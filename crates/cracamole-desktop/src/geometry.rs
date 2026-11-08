//! 將置中、可能被裁切的遠端影格座標映射到原生視窗。

/// `clamp` 僅供已捕捉滑鼠的拖曳及放開使用；一般移動與按下須忽略留白區。
pub fn centered_image_point(
    viewport: (u32, u32),
    frame: (u32, u32),
    point: (i32, i32),
    clamp: bool,
) -> Option<(u32, u32)> {
    fn axis(viewport: u32, frame: u32, coordinate: i32, clamp: bool) -> Option<u32> {
        if viewport == 0 || frame == 0 {
            return None;
        }
        let offset = (i64::from(viewport) - i64::from(frame)).div_euclid(2);
        let position = i64::from(coordinate);
        if !clamp && !(0..i64::from(viewport)).contains(&position) {
            return None;
        }
        let in_view = position.clamp(0, i64::from(viewport) - 1);
        let remote = in_view - offset;
        if !clamp && !(0..i64::from(frame)).contains(&remote) {
            return None;
        }
        Some(remote.clamp(0, i64::from(frame) - 1) as u32)
    }
    Some((
        axis(viewport.0, frame.0, point.0, clamp)?,
        axis(viewport.1, frame.1, point.1, clamp)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_centered_images_and_ignores_margins() {
        assert_eq!(
            centered_image_point((800, 600), (400, 200), (200, 200), false),
            Some((0, 0))
        );
        assert_eq!(
            centered_image_point((800, 600), (400, 200), (599, 399), false),
            Some((399, 199))
        );
        assert_eq!(
            centered_image_point((800, 600), (400, 200), (199, 200), false),
            None
        );
        assert_eq!(
            centered_image_point((800, 600), (400, 200), (600, 200), false),
            None
        );
        assert_eq!(
            centered_image_point((800, 600), (400, 200), (-50, 700), true),
            Some((0, 199))
        );
    }

    #[test]
    fn maps_clipped_images_and_dragging_to_visible_edges() {
        assert_eq!(
            centered_image_point((400, 200), (800, 600), (0, 0), false),
            Some((200, 200))
        );
        assert_eq!(
            centered_image_point((400, 200), (800, 600), (399, 199), false),
            Some((599, 399))
        );
        assert_eq!(
            centered_image_point((400, 200), (800, 600), (500, -20), true),
            Some((599, 200))
        );
        assert_eq!(
            centered_image_point((400, 200), (800, 600), (400, 0), false),
            None
        );
        assert_eq!(
            centered_image_point((0, 200), (800, 600), (0, 0), true),
            None
        );
        assert_eq!(
            centered_image_point((400, 200), (0, 600), (0, 0), false),
            None
        );
        // 奇數尺寸的裁切位置依 Windows 實際控制項繪製方向取整。
        assert_eq!(
            centered_image_point((4, 4), (5, 5), (0, 0), false),
            Some((1, 1))
        );
        assert_eq!(centered_image_point((5, 5), (4, 4), (4, 2), false), None);
    }
}
