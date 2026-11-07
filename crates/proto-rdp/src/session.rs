//! Active RDP session: server graphics -> Guacamole display, client input -> RDP input.

use crate::clipboard::{self, ClipEvent};
use crate::connect::{establish, Framed};
use crate::image::Image;
use crate::keymap::{self, Key};
use crate::settings::RdpSettings;
use base64::Engine as _;
use guac_display::Display;
use guac_protocol::Instruction;
use guac_session::{connect_tcp, InputEvent, SessionCtx, SessionError, Status};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse};
use ironrdp_cliprdr::{CliprdrClient, CliprdrSvcMessages};
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationSequence, ConnectionActivationState,
};
use ironrdp_input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::{single_sequence_step, FramedWrite};
use std::time::Duration;
use tokio::time::Instant;

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const HEARTBEAT: Duration = Duration::from_secs(5);
/// Window resizes are coalesced: the desktop is resized once the browser size settles.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(300);
const RESIZE_RETRY: Duration = Duration::from_millis(500);

fn session_error(e: impl std::fmt::Display) -> SessionError {
    SessionError::plugin(Status::UpstreamError, format!("RDP session error: {e}"))
}

pub async fn run_rdp(mut ctx: SessionCtx, settings: RdpSettings) -> Result<(), SessionError> {
    let caps = ctx.tx.caps().clone();
    let (clip_tx, mut clip_rx) = tokio::sync::mpsc::unbounded_channel();
    let clipboard_enabled = !(settings.disable_copy && settings.disable_paste);
    let connecting = async {
        let stream = connect_tcp(&settings.hostname, settings.port, settings.timeout_secs).await?;
        tracing::info!("RDP TCP connection successful.");
        establish(
            &settings,
            stream,
            caps.optimal_width,
            caps.optimal_height,
            clipboard_enabled.then_some(clip_tx),
        )
        .await
    };
    let (mut framed, result) = tokio::select! {
        _ = ctx.tx.cancelled() => return Ok(()),
        result = tokio::time::timeout(Duration::from_secs(settings.timeout_secs), connecting) =>
            result.map_err(|_| SessionError::plugin(Status::UpstreamTimeout, "RDP handshake timed out."))??,
    };
    tracing::info!(
        "RDP session established ({}x{}).",
        result.desktop_size.width,
        result.desktop_size.height
    );

    let factory = result.activation_factory.clone();
    let (io_channel_id, user_channel_id) = (result.io_channel_id, result.user_channel_id);
    let (width, height) = (result.desktop_size.width, result.desktop_size.height);
    let mut image = Image::new(width, height)?;
    let mut stage: ActiveStage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: None,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();

    let mut display = Display::new(u32::from(width), u32::from(height));
    let mut input = Database::new();
    let mut buttons: u32 = 0;
    let mut dirty = true;
    let mut last_frame: Option<Instant> = None;
    let mut heartbeat = tokio::time::interval_at(Instant::now() + HEARTBEAT, HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending: Vec<ironrdp_pdu::geometry::InclusiveRectangle> = vec![];
    let mut desktop = (width, height);
    let mut local_clipboard: Option<String> = None;
    let mut clip_stream: u32 = 0x4000_0000;
    let mut resize: Option<((u32, u32), Instant)> = None;

    loop {
        let resize_due = async {
            match resize {
                Some((_, t)) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        let next_frame = match (dirty, last_frame) {
            (true, Some(t)) => Some(t + FRAME_INTERVAL),
            (true, None) => Some(Instant::now()),
            _ => None,
        };
        let frame_due = async {
            match next_frame {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            biased;
            _ = ctx.tx.cancelled() => {
                graceful_shutdown(&mut stage, &mut framed).await;
                return Ok(());
            }
            event = ctx.events.recv() => {
                let Some(event) = event else { return Ok(()) };
                if let InputEvent::Resize { width, height } = event {
                    let target = (width.clamp(200, crate::image::MAX_DIMENSION) & !1,
                        height.clamp(200, crate::image::MAX_DIMENSION));
                    if target.0 >= 200 && target.1 >= 200 && target != (u32::from(desktop.0), u32::from(desktop.1)) {
                        resize = Some((target, Instant::now() + RESIZE_DEBOUNCE));
                    }
                    continue;
                }
                if let InputEvent::Clipboard(text) = event {
                    if !settings.read_only && !settings.disable_paste && clipboard_enabled {
                        local_clipboard = Some(clipboard::to_remote(&text));
                        clip_call(&mut stage, &mut framed, |c| {
                            c.initiate_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)])
                        })
                        .await?;
                    }
                    continue;
                }
                if settings.read_only {
                    continue;
                }
                let shift_down = input.is_key_pressed(SHIFT_L) || input.is_key_pressed(SHIFT_R);
                let operations = operations_for(event, &mut buttons, shift_down);
                if operations.is_empty() {
                    continue;
                }
                let events = input.apply(operations);
                if events.is_empty() {
                    continue;
                }
                let outputs = stage
                    .process_fastpath_input(&mut image, &events)
                    .map_err(session_error)?;
                match handle_outputs(outputs, &mut framed, &mut pending).await? {
                    Flow::Continue => {}
                    Flow::Terminate => return Ok(()),
                    Flow::Reactivate => {
                        let size = reactivate(&mut stage, &mut framed, &factory, io_channel_id, user_channel_id).await?;
                        image = Image::new(size.0, size.1)?;
                        display.resize(u32::from(size.0), u32::from(size.1));
                        desktop = size;
                        dirty = true;
                    }
                }
                dirty |= !pending.is_empty();
            }
            frame = framed.read_pdu() => {
                let (action, payload) = frame.map_err(|e| {
                    SessionError::plugin(Status::UpstreamUnavailable, format!("RDP connection lost: {e}"))
                })?;
                let received_at = framed.last_read_at();
                let outputs = stage
                    .process(&mut image, action, &payload, received_at)
                    .map_err(session_error)?;
                match handle_outputs(outputs, &mut framed, &mut pending).await? {
                    Flow::Continue => {}
                    Flow::Terminate => return Ok(()),
                    Flow::Reactivate => {
                        let size = reactivate(&mut stage, &mut framed, &factory, io_channel_id, user_channel_id).await?;
                        image = Image::new(size.0, size.1)?;
                        display.resize(u32::from(size.0), u32::from(size.1));
                        desktop = size;
                        dirty = true;
                    }
                }
                dirty |= !pending.is_empty();
            }
            Some(clip_event) = clip_rx.recv() => {
                match clip_event {
                    ClipEvent::FormatListRequested => {
                        let formats: &[ClipboardFormat] = if local_clipboard.is_some() {
                            &[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]
                        } else {
                            &[]
                        };
                        clip_call(&mut stage, &mut framed, |c| c.initiate_copy(formats)).await?;
                    }
                    ClipEvent::RemoteCopied(true) if !settings.disable_copy => {
                        clip_call(&mut stage, &mut framed, |c| {
                            c.initiate_paste(ClipboardFormatId::CF_UNICODETEXT)
                        })
                        .await?;
                    }
                    ClipEvent::RemoteCopied(_) => {}
                    ClipEvent::DataRequested(format) => {
                        let response = match (&local_clipboard, format == ClipboardFormatId::CF_UNICODETEXT) {
                            (Some(text), true) => FormatDataResponse::new_unicode_string(text),
                            _ => FormatDataResponse::new_error(),
                        };
                        clip_call(&mut stage, &mut framed, |c| c.submit_format_data(response)).await?;
                    }
                    ClipEvent::RemoteText(text) => {
                        if !settings.disable_copy {
                            send_clipboard(&ctx, clip_stream, &text).await?;
                            clip_stream += 1;
                        }
                    }
                }
            }
            _ = resize_due => {
                if let Some(((w, h), _)) = resize.take() {
                    match stage.encode_resize(w, h, None, None) {
                        Some(Ok(frame)) => {
                            tracing::debug!("requesting RDP desktop resize to {w}x{h}");
                            framed.write_all(&frame).await.map_err(|e| {
                                SessionError::plugin(Status::UpstreamUnavailable, format!("RDP write failed: {e}"))
                            })?;
                        }
                        // The Display Control channel is still opening; try again shortly.
                        None => resize = Some(((w, h), Instant::now() + RESIZE_RETRY)),
                        Some(Err(e)) => tracing::warn!("RDP resize request failed: {e}"),
                    }
                }
            }
            _ = heartbeat.tick() => {
                ctx.tx.sync().await?;
            }
            _ = frame_due => {
                paint(&mut display, &image, &mut pending);
                let out = display.flush();
                if !out.is_empty() {
                    for ins in out {
                        ctx.tx.send(ins).await?;
                    }
                    ctx.tx.sync().await?;
                    last_frame = Some(Instant::now());
                    tokio::select! {
                        _ = ctx.tx.wait_lag(2) => {}
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {
                            tracing::warn!("client did not acknowledge frames for 10s; continuing");
                        }
                    }
                }
                dirty = false;
            }
        }
    }
}

/// Run a CLIPRDR operation and write the resulting channel frame to the server.
async fn clip_call(
    stage: &mut ActiveStage,
    framed: &mut Framed,
    operation: impl FnOnce(
        &mut CliprdrClient,
    ) -> ironrdp_pdu::PduResult<CliprdrSvcMessages<ironrdp_cliprdr::Client>>,
) -> Result<(), SessionError> {
    let Some(channel) = stage.get_svc_processor_mut::<CliprdrClient>() else {
        return Ok(());
    };
    let messages = operation(channel).map_err(session_error)?;
    let frame = stage
        .process_svc_processor_messages(messages)
        .map_err(session_error)?;
    framed.write_all(&frame).await.map_err(|e| {
        SessionError::plugin(
            Status::UpstreamUnavailable,
            format!("RDP write failed: {e}"),
        )
    })
}

/// Send remote clipboard text to the Guacamole client (3-byte aligned chunks keep base64 unpadded).
async fn send_clipboard(ctx: &SessionCtx, stream: u32, text: &str) -> Result<(), SessionError> {
    let id = stream.to_string();
    ctx.tx
        .send(Instruction::new(
            "clipboard",
            [id.clone(), "text/plain".into()],
        ))
        .await?;
    for chunk in text.as_bytes().chunks(3072) {
        let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
        ctx.tx
            .send(Instruction::new("blob", [id.clone(), b64]))
            .await?;
    }
    ctx.tx.send(Instruction::new("end", [id])).await
}

enum Flow {
    Continue,
    Terminate,
    Reactivate,
}

/// Deactivation-Reactivation Sequence: renegotiate capabilities after a server-side resize.
async fn reactivate(
    stage: &mut ActiveStage,
    framed: &mut Framed,
    factory: &ConnectionActivationFactory,
    io_channel_id: u16,
    user_channel_id: u16,
) -> Result<(u16, u16), SessionError> {
    let mut sequence: ConnectionActivationSequence = factory.create();
    let mut buf = ironrdp_core::WriteBuf::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        tokio::time::timeout_at(
            deadline,
            single_sequence_step(framed, &mut sequence, &mut buf),
        )
        .await
        .map_err(|_| SessionError::plugin(Status::UpstreamTimeout, "RDP reactivation timed out."))?
        .map_err(session_error)?;
        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            static_channel_chunk_size,
            enable_server_pointer,
            pointer_software_rendering,
            ..
        } = sequence.connection_activation_state()
        {
            if !stage.reactivate(
                io_channel_id,
                user_channel_id,
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                return Err(session_error("invalid virtual channel chunk size"));
            }
            tracing::info!(
                "RDP desktop is now {}x{}",
                desktop_size.width,
                desktop_size.height
            );
            return Ok((desktop_size.width, desktop_size.height));
        }
    }
}

/// Writes response frames to the server and reports whether the session continues.
async fn handle_outputs(
    outputs: Vec<ActiveStageOutput>,
    framed: &mut Framed,
    pending: &mut Vec<ironrdp_pdu::geometry::InclusiveRectangle>,
) -> Result<Flow, SessionError> {
    for output in outputs {
        match output {
            ActiveStageOutput::ResponseFrame(frame) => {
                framed.write_all(&frame).await.map_err(|e| {
                    SessionError::plugin(
                        Status::UpstreamUnavailable,
                        format!("RDP write failed: {e}"),
                    )
                })?;
            }
            ActiveStageOutput::GraphicsUpdate(rect) => pending.push(rect),
            ActiveStageOutput::Terminate(reason) => {
                tracing::info!("RDP server ended the session: {reason}");
                return Ok(Flow::Terminate);
            }
            ActiveStageOutput::DeactivateAll => return Ok(Flow::Reactivate),
            _ => {}
        }
    }
    Ok(Flow::Continue)
}

async fn graceful_shutdown(stage: &mut ActiveStage, framed: &mut Framed) {
    if let Ok(outputs) = stage.graceful_shutdown() {
        for output in outputs {
            if let ActiveStageOutput::ResponseFrame(frame) = output {
                let _ = framed.write_all(&frame).await;
            }
        }
    }
}

/// Copy updated rectangles from the decoded RGBA image into the Guacamole framebuffer.
fn paint(
    display: &mut Display,
    image: &DecodedImage,
    pending: &mut Vec<ironrdp_pdu::geometry::InclusiveRectangle>,
) {
    let (iw, ih) = (usize::from(image.width()), usize::from(image.height()));
    let data = image.data();
    for rect in pending.drain(..) {
        let (x0, y0) = (usize::from(rect.left), usize::from(rect.top));
        let (x1, y1) = (
            (usize::from(rect.right) + 1).min(iw),
            (usize::from(rect.bottom) + 1).min(ih),
        );
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        for y in y0..y1 {
            let row = y * iw * 4;
            for x in x0..x1 {
                let o = row + x * 4;
                if let Some(px) = data.get(o..o + 3) {
                    display.put_pixel_raw(x as u32, y as u32, [px[0], px[1], px[2], 255]);
                }
            }
        }
        display.mark_dirty(guac_display::Rect::new(
            x0 as i32,
            y0 as i32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        ));
    }
}

const SHIFT_L: Scancode = Scancode::from_u8(false, 0x2a);
const SHIFT_R: Scancode = Scancode::from_u8(false, 0x36);

fn operations_for(event: InputEvent, buttons: &mut u32, shift_down: bool) -> Vec<Operation> {
    match event {
        InputEvent::Key { keysym, pressed } => match keymap::map(keysym) {
            Some(Key::Scan(code)) if pressed => {
                // Clients that send a shifted symbol without a Shift key event (on-screen
                // keyboards, scripted input) still get the right character.
                if keymap::needs_shift(keysym) && !shift_down {
                    vec![
                        Operation::KeyPressed(SHIFT_L),
                        Operation::KeyPressed(code),
                        Operation::KeyReleased(code),
                        Operation::KeyReleased(SHIFT_L),
                    ]
                } else {
                    vec![Operation::KeyPressed(code)]
                }
            }
            Some(Key::Scan(code)) => vec![Operation::KeyReleased(code)],
            Some(Key::Unicode(c)) => vec![if pressed {
                Operation::UnicodeKeyPressed(c)
            } else {
                Operation::UnicodeKeyReleased(c)
            }],
            None => vec![],
        },
        InputEvent::Mouse { x, y, mask } => {
            let mut ops = vec![Operation::MouseMove(MousePosition {
                x: x.clamp(0, i32::from(u16::MAX)) as u16,
                y: y.clamp(0, i32::from(u16::MAX)) as u16,
            })];
            let changed = *buttons ^ mask;
            for (bit, button) in [
                (1, MouseButton::Left),
                (2, MouseButton::Middle),
                (4, MouseButton::Right),
            ] {
                if changed & bit != 0 {
                    ops.push(if mask & bit != 0 {
                        Operation::MouseButtonPressed(button)
                    } else {
                        Operation::MouseButtonReleased(button)
                    });
                }
            }
            // Wheel "buttons" fire on the press edge only.
            for (bit, units) in [(8, 120i16), (16, -120i16)] {
                if changed & bit != 0 && mask & bit != 0 {
                    ops.push(Operation::WheelRotations(WheelRotations {
                        is_vertical: true,
                        rotation_units: units,
                    }));
                }
            }
            *buttons = mask;
            ops
        }
        InputEvent::Resize { .. } | InputEvent::Clipboard(_) => vec![],
    }
}
