//! Headless proof that ApplicationWindow forwards file-operation shortcuts to the
//! host, and that text-editing surfaces (inline rename, address bar) are not hijacked.

use std::cell::RefCell;
use std::rc::Rc;

use fx_ui::{ApplicationWindow, FileRow};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

type Seen = Rc<RefCell<Vec<(String, bool, bool, bool)>>>;

fn window_with_row() -> (ApplicationWindow, Seen) {
    let window = ApplicationWindow::new().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(800.0, 600.0));
    let row = FileRow {
        id: 1,
        display_name: SharedString::from("a.txt"),
        kind: SharedString::from("File"),
        is_folder: false,
        size_text: SharedString::default(),
        modified_text: SharedString::default(),
        type_text: SharedString::default(),
        attributes_text: SharedString::default(),
        selected: false,
        icon_key: 0,
    };
    window.set_rows(ModelRc::new(VecModel::from(vec![row])));
    let seen: Seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = seen.clone();
    window.on_shortcut_key(move |text, ctrl, shift, alt| {
        recorder
            .borrow_mut()
            .push((text.to_string(), ctrl, shift, alt));
        true
    });
    (window, seen)
}

fn click(window: &ApplicationWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn press(window: &ApplicationWindow, text: &str) {
    let text = SharedString::from(text);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Presses a modified shortcut the way the OS delivers it: the modifier is held
/// down while the key itself is pressed, and released only afterwards.
fn combo(window: &ApplicationWindow, modifier: &str, main: &str) {
    let m = SharedString::from(modifier);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: m.clone() });
    let t = SharedString::from(main);
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: t.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: t });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: m });
}

fn key(k: Key) -> String {
    String::from(char::from(k))
}

#[test]
fn ctrl_c_and_delete_reach_the_host() {
    i_slint_backend_testing::init_no_event_loop();
    let (window, seen) = window_with_row();
    let clicks: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
    let click_recorder = clicks.clone();
    window.on_row_selected(move |id, _ctrl, _shift| {
        click_recorder.borrow_mut().push(id);
    });
    // Click the first row: the list starts below address (30) + breadcrumb (28) + header (28).
    click(&window, 100.0, 95.0);
    assert_eq!(
        clicks.borrow().as_slice(),
        &[1],
        "the click did not land on the first row"
    );

    combo(&window, &key(Key::Control), "c");
    assert!(
        seen.borrow()
            .iter()
            .any(|(t, c, s, a)| t == "c" && *c && !*s && !*a),
        "Ctrl+C was not forwarded: {:?}",
        seen.borrow()
    );

    press(&window, &key(Key::Delete));
    assert!(
        seen.borrow()
            .iter()
            .any(|(t, c, s, a)| t.as_str() == key(Key::Delete) && !*c && !*s && !*a),
        "Delete was not forwarded: {:?}",
        seen.borrow()
    );
}

#[test]
fn text_editing_surfaces_are_not_hijacked() {
    i_slint_backend_testing::init_no_event_loop();
    let (window, seen) = window_with_row();
    click(&window, 100.0, 95.0);

    // Inline rename active: file shortcuts must not fire.
    window.set_status_rename_target_id(1);
    let before = seen.borrow().len();
    press(&window, &key(Key::Delete));
    assert_eq!(
        seen.borrow().len(),
        before,
        "Delete was forwarded during an inline rename"
    );
    window.set_status_rename_target_id(-1);

    // Address bar focused: F2 must not start a file rename.
    click(&window, 100.0, 15.0);
    let before = seen.borrow().len();
    press(&window, &key(Key::F2));
    assert_eq!(
        seen.borrow().len(),
        before,
        "F2 was forwarded while the address bar has focus"
    );
}

#[test]
fn delete_confirmation_cancels_on_escape_without_deleting() {
    i_slint_backend_testing::init_no_event_loop();
    let (window, seen) = window_with_row();
    let answers: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let rec_cancel = answers.clone();
    window.on_permanent_delete_cancelled(move || {
        rec_cancel.borrow_mut().push("cancelled".into());
    });
    let rec_confirm = answers.clone();
    window.on_permanent_delete_confirmed(move || {
        rec_confirm.borrow_mut().push("confirmed".into());
    });

    // Escape with no dialog open must not answer anything.
    press(&window, &key(Key::Escape));
    assert!(
        answers.borrow().is_empty(),
        "Escape answered a confirmation that was never shown"
    );

    // Opened: Escape closes the overlay and answers cancel — never confirm.
    let forwarded = seen.borrow().len();
    window.set_permanent_delete_count_text(SharedString::from("3 items"));
    window.set_confirm_permanent_delete(true);
    press(&window, &key(Key::Escape));
    assert_eq!(
        answers.borrow().as_slice(),
        &["cancelled".to_string()],
        "Escape must cancel the pending permanent delete"
    );
    assert!(
        !window.get_confirm_permanent_delete(),
        "the overlay must hide after Escape"
    );
    assert_eq!(
        seen.borrow().len(),
        forwarded,
        "keys must not forward file shortcuts while the confirmation is up"
    );
}
