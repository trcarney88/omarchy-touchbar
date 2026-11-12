#[path = "../src/linux/display.rs"]
mod display;

#[test]
fn accepts_only_exact_panel_mode_and_connector() {
    assert!(display::validate_display("DSI-1", 60, 2008).is_ok());
    for mode in [(1920, 1080), (60, 2007)] {
        assert!(display::validate_display("DSI-1", mode.0, mode.1).is_err());
    }
    assert!(display::validate_display("eDP-1", 60, 2008).is_err());
}

#[test]
fn rotates_corners_and_preserves_row_padding() {
    let mut src = vec![0u8; display::SOURCE_WIDTH * display::SOURCE_HEIGHT * 4];
    let at = |x: usize, y: usize| (y * display::SOURCE_WIDTH + x) * 4;
    src[at(0, display::SOURCE_HEIGHT - 1)..at(0, display::SOURCE_HEIGHT - 1) + 4]
        .copy_from_slice(&[1, 2, 3, 255]);
    src[at(0, 0)..at(0, 0) + 4].copy_from_slice(&[4, 5, 6, 255]);
    let pitch = 256;
    let mut dst = vec![0xa5; pitch * display::HEIGHT];
    display::rgba_to_rotated_xrgb(&src, pitch, &mut dst).unwrap();
    assert_eq!(&dst[..4], &[3, 2, 1, 0]);
    let last = (display::WIDTH - 1) * 4;
    assert_eq!(&dst[last..last + 4], &[6, 5, 4, 0]);
    assert!(dst[display::WIDTH * 4..pitch].iter().all(|b| *b == 0xa5));
}

#[test]
fn rejects_short_frames_and_mappings() {
    let mut dst = vec![0; 256 * display::HEIGHT];
    assert!(display::rgba_to_rotated_xrgb(&[], 256, &mut dst).is_err());
    let src = vec![0; display::SOURCE_WIDTH * display::SOURCE_HEIGHT * 4];
    assert!(display::rgba_to_rotated_xrgb(&src, 239, &mut dst).is_err());
    let short_len = dst.len() - 1;
    assert!(display::rgba_to_rotated_xrgb(&src, 256, &mut dst[..short_len]).is_err());
}
