#[path = "../src/linux/keyboard.rs"]
mod keyboard;
#[path = "../src/linux/lease.rs"]
mod lease;

#[test]
fn connector_and_mode_guards_are_strict() {
    assert!(lease::choose_connector(["HDMI-A-1", "DSI-1"]).is_ok());
    assert!(lease::choose_connector(["HDMI-A-1"]).is_err());
    assert!(lease::choose_connector(["DSI-1", "DSI-1"]).is_err());
    assert!(lease::validate_mode(60, 2008, 60_000).is_ok());
    assert!(lease::validate_mode(60, 2008, 59_940).is_ok());
    assert!(lease::validate_mode(1920, 1080, 60_000).is_err());
}

#[test]
fn rotates_rgba_into_padded_xrgb_and_rejects_short_buffers() {
    let mut source = vec![0; 2008 * 60 * 4];
    for y in 0..60 {
        for x in 0..2008 {
            let i = (y * 2008 + x) * 4;
            source[i..i + 4].copy_from_slice(&[x as u8, y as u8, 17, 255]);
        }
    }
    let pitch = 64 * 4;
    let mut output = vec![0xaa; pitch * 2008];
    lease::rgba_to_rotated_xrgb(&source, pitch, &mut output).unwrap();
    assert_eq!(&output[0..4], &[17, 59, 0, 0]);
    assert_eq!(&output[(59 * 4)..(60 * 4)], &[17, 0, 0, 0]);
    assert!(output[60 * 4..pitch].iter().all(|b| *b == 0xaa));
    assert!(lease::rgba_to_rotated_xrgb(&source[..10], pitch, &mut output).is_err());
    assert!(lease::rgba_to_rotated_xrgb(&source, 4, &mut output).is_err());
}

#[test]
fn revocation_closes_ready_state_and_keys_are_limited() {
    let mut state = lease::LeaseState::default();
    assert!(!state.ready());
    state.mark_ready();
    assert!(state.ready());
    state.revoke();
    assert!(!state.ready());
    let mut keys = keyboard::PressedKeys::default();
    assert_eq!(keys.transition(12, true), Some((88, true)));
    assert_eq!(keys.transition(12, false), Some((88, false)));
    assert_eq!(keys.transition(13, true), None);
}
