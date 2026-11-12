#[path = "../src/linux/keyboard.rs"]
mod keyboard;

#[test]
fn function_key_codes_and_pressed_state_are_bounded_and_balanced() {
    assert_eq!(keyboard::function_key_code(1), Some(59));
    assert_eq!(keyboard::function_key_code(10), Some(68));
    assert_eq!(keyboard::function_key_code(11), Some(87));
    assert_eq!(keyboard::function_key_code(12), Some(88));
    assert_eq!(keyboard::function_key_code(0), None);
    assert_eq!(keyboard::function_key_code(13), None);

    let mut keys = keyboard::PressedKeys::default();
    assert_eq!(keys.transition(1, true), Some((59, true)));
    assert_eq!(keys.transition(1, true), None);
    assert_eq!(keys.transition(2, false), None);
    assert_eq!(keys.release_all(), vec![59]);
    assert_eq!(keys.release_all(), Vec::<u32>::new());
}
