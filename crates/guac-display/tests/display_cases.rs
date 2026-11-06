use guac_display::{Display, Rect, MAX_BLOB_BYTES};
use guac_testkit::DisplayReplay;

// ---- DP-RECT ----
#[test]
fn rect_intersection_and_touching() {
    let a = Rect::new(0, 0, 10, 10);
    assert!(a.intersects(&Rect::new(5, 5, 10, 10)));
    assert_eq!(
        a.intersection(&Rect::new(5, 5, 10, 10)),
        Rect::new(5, 5, 5, 5)
    );
    // edge-touching rectangles do not intersect
    assert!(!a.intersects(&Rect::new(10, 0, 5, 5)));
    assert!(!a.intersects(&Rect::new(0, 10, 5, 5)));
}

#[test]
fn rect_empty_and_negative() {
    let a = Rect::new(0, 0, 10, 10);
    assert!(!a.intersects(&Rect::new(5, 5, 0, 5)));
    assert_eq!(
        a.intersection(&Rect::new(-5, -5, 10, 10)),
        Rect::new(0, 0, 5, 5)
    );
    assert_eq!(a.union(&Rect::new(0, 0, 0, 0)), a);
}

#[test]
fn rect_no_overflow_at_extremes() {
    let big = Rect::new(i32::MAX - 1, i32::MAX - 1, u32::MAX, u32::MAX);
    let _ = big.right();
    let _ = big.intersection(&Rect::new(0, 0, 10, 10));
    let _ = big.union(&Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX));
}

#[test]
fn rect_union() {
    let u = Rect::new(0, 0, 5, 5).union(&Rect::new(10, 10, 5, 5));
    assert_eq!(u, Rect::new(0, 0, 15, 15));
}

// ---- DP-DIRTY ----
fn clean(d: &mut Display) {
    d.flush();
}

#[test]
fn dirty_01_single_small_change() {
    let mut d = Display::new(100, 100);
    clean(&mut d);
    d.fill_rect(Rect::new(10, 20, 5, 3), [1, 2, 3, 255]);
    assert_eq!(d.dirty_bands(), vec![Rect::new(10, 20, 5, 3)]);
}

#[test]
fn dirty_02_adjacent_rows_merge() {
    let mut d = Display::new(100, 100);
    clean(&mut d);
    d.fill_rect(Rect::new(10, 10, 5, 2), [1, 1, 1, 255]);
    d.fill_rect(Rect::new(20, 12, 5, 2), [1, 1, 1, 255]);
    assert_eq!(d.dirty_bands(), vec![Rect::new(10, 10, 15, 4)]);
}

#[test]
fn dirty_03_separate_bands_stay_separate() {
    let mut d = Display::new(100, 100);
    clean(&mut d);
    d.fill_rect(Rect::new(0, 0, 100, 2), [1, 1, 1, 255]);
    d.fill_rect(Rect::new(0, 90, 100, 2), [1, 1, 1, 255]);
    assert_eq!(d.dirty_bands().len(), 2);
}

#[test]
fn dirty_04_flush_clears_and_second_flush_is_empty() {
    let mut d = Display::new(50, 50);
    assert!(!d.flush().is_empty());
    assert!(!d.is_dirty());
    assert!(d.flush().is_empty());
}

#[test]
fn dirty_05_resize_marks_all_and_sends_size_first() {
    let mut d = Display::new(50, 50);
    clean(&mut d);
    d.resize(80, 60);
    let out = d.flush();
    assert_eq!(out[0].opcode, "size");
    assert_eq!(out[0].args, vec!["0", "80", "60"]);
    assert_eq!(out[1].opcode, "img");
}

#[test]
fn dirty_out_of_bounds_is_clipped() {
    let mut d = Display::new(10, 10);
    clean(&mut d);
    d.fill_rect(Rect::new(-50, -50, 1000, 1000), [9, 9, 9, 255]);
    assert_eq!(d.dirty_bands(), vec![Rect::new(0, 0, 10, 10)]);
    d.flush();
    d.fill_rect(Rect::new(500, 500, 5, 5), [9, 9, 9, 255]);
    assert!(!d.is_dirty());
}

// ---- DP-FLUSH: round-trip through the replay client ----
#[test]
fn flush_02_png_roundtrip_pixels() {
    let mut d = Display::new(64, 32);
    d.fill_rect(Rect::new(0, 0, 64, 32), [10, 20, 30, 255]);
    d.fill_rect(Rect::new(5, 6, 7, 8), [200, 100, 50, 255]);
    let mut rp = DisplayReplay::new();
    for i in d.flush() {
        rp.apply(&i);
    }
    assert!(rp.unsupported.is_empty(), "{:?}", rp.unsupported);
    assert_eq!(rp.size(0), (64, 32));
    assert_eq!(rp.pixel(0, 0, 0), Some([10, 20, 30, 255]));
    assert_eq!(rp.pixel(0, 5, 6), Some([200, 100, 50, 255]));
    assert_eq!(rp.pixel(0, 11, 13), Some([200, 100, 50, 255]));
    assert_eq!(rp.pixel(0, 12, 14), Some([10, 20, 30, 255]));
}

#[test]
fn flush_03_incremental_updates_only_changed_region() {
    let mut d = Display::new(64, 64);
    d.fill_rect(Rect::new(0, 0, 64, 64), [1, 1, 1, 255]);
    let mut rp = DisplayReplay::new();
    for i in d.flush() {
        rp.apply(&i);
    }
    d.fill_rect(Rect::new(10, 10, 4, 4), [255, 0, 0, 255]);
    let out = d.flush();
    assert!(
        out.iter().all(|i| i.opcode != "size"),
        "no size without resize"
    );
    let img = out.iter().find(|i| i.opcode == "img").unwrap();
    assert_eq!(&img.args[4..], ["10", "10"]);
    for i in &out {
        rp.apply(i);
    }
    assert_eq!(rp.pixel(0, 10, 10), Some([255, 0, 0, 255]));
    assert_eq!(rp.pixel(0, 9, 9), Some([1, 1, 1, 255]));
}

#[test]
fn flush_blob_chunking_respects_limit() {
    use base64::Engine;
    let mut d = Display::new(300, 300);
    // noisy content defeats PNG compression, forcing several blobs
    let mut x = 12345u32;
    for y in 0..300 {
        for xx in 0..300 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            d.put_pixel_raw(
                xx,
                y,
                [(x >> 8) as u8, (x >> 16) as u8, (x >> 24) as u8, 255],
            );
        }
    }
    d.mark_dirty(Rect::new(0, 0, 300, 300));
    let out = d.flush();
    let blobs: Vec<_> = out.iter().filter(|i| i.opcode == "blob").collect();
    assert!(blobs.len() > 1);
    for b in &blobs {
        let n = base64::engine::general_purpose::STANDARD
            .decode(&b.args[1])
            .unwrap()
            .len();
        assert!(n <= MAX_BLOB_BYTES);
    }
    // every img has a matching end
    let imgs = out.iter().filter(|i| i.opcode == "img").count();
    assert_eq!(imgs, out.iter().filter(|i| i.opcode == "end").count());
}

#[test]
fn copy_within_overlapping_scroll() {
    let mut d = Display::new(10, 10);
    for y in 0..10u32 {
        d.fill_rect(Rect::new(0, y as i32, 10, 1), [y as u8, 0, 0, 255]);
    }
    d.flush();
    // scroll up by 2 rows
    d.copy_within(Rect::new(0, 2, 10, 8), 0, 0);
    assert_eq!(d.pixel(0, 0), Some([2, 0, 0, 255]));
    assert_eq!(d.pixel(0, 7), Some([9, 0, 0, 255]));
    assert_eq!(d.dirty_bands(), vec![Rect::new(0, 0, 10, 8)]);
}

#[test]
fn stream_ids_are_unique_within_flush() {
    let mut d = Display::new(20, 20);
    d.flush();
    d.fill_rect(Rect::new(0, 0, 20, 2), [1, 1, 1, 255]);
    d.fill_rect(Rect::new(0, 10, 20, 2), [1, 1, 1, 255]);
    let ids: Vec<_> = d
        .flush()
        .into_iter()
        .filter(|i| i.opcode == "img")
        .map(|i| i.args[0].clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn size_is_bounded_so_a_client_cannot_exhaust_memory() {
    use guac_display::MAX_DIMENSION;
    let d = Display::new(u32::MAX, 1);
    assert_eq!(d.size(), (MAX_DIMENSION, 1));
    let mut d = Display::new(10, 10);
    d.resize(0, u32::MAX / 2);
    assert_eq!(d.size(), (1, MAX_DIMENSION));
    d.fill_rect(
        Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX),
        [1, 2, 3, 255],
    );
    let _ = d.flush();
}
