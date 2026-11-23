//! Headless proof that the frameless window's caption buttons are laid out at the right
//! edge of the band and reach the host through their own callbacks.
//!
//! The Win32 side (dragging the band, the resize edges, the maximise state) cannot run
//! without a window, so it stays a manual check; this test pins the geometry the Win32
//! hit-test mirrors (46 px buttons, 34 px band, three of them at the right).

use std::cell::RefCell;
use std::rc::Rc;

use fx_ui::ApplicationWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

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

#[test]
fn caption_buttons_open_at_the_right_edge_of_the_band() {
    let window = ApplicationWindow::new().unwrap();
    assert!(!window.get_window_maximized());
    window
        .window()
        .set_size(slint::LogicalSize::new(800.0, 600.0));

    let hits: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let recorder = hits.clone();
    window.on_window_minimize(move || recorder.borrow_mut().push("minimize"));
    let recorder = hits.clone();
    window.on_window_toggle_maximize(move || recorder.borrow_mut().push("maximize"));
    let recorder = hits.clone();
    window.on_window_close(move || recorder.borrow_mut().push("close"));

    // Three 46 px buttons end flush with the right edge; y = 17 is the middle of the
    // 34 px band.
    click(&window, 800.0 - 23.0, 17.0);
    click(&window, 800.0 - 69.0, 17.0);
    click(&window, 800.0 - 115.0, 17.0);

    assert_eq!(&*hits.borrow(), &["close", "maximize", "minimize"]);

    // The restore glyph follows the state the shell reports after a native maximise.
    window.set_window_maximized(true);
    assert!(window.get_window_maximized());
}
