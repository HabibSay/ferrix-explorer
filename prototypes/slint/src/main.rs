slint::include_modules!();

use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};

fn main() -> Result<(), slint::PlatformError> {
    let all_rows = (0..100_000)
        .map(|index| SharedString::from(format!("Ferrix sample item {index:06}")))
        .collect::<Vec<_>>();
    let source = Rc::new(all_rows);
    let visible = Rc::new(VecModel::from(source.as_ref().clone()));

    let window = MainWindow::new()?;
    let model: ModelRc<SharedString> = visible.clone().into();
    window.set_rows(model);

    let filter_source = source.clone();
    let filter_model = visible.clone();
    window.on_filter_changed(move |filter| {
        let query = filter.to_lowercase();
        let rows = filter_source
            .iter()
            .filter(|row| row.to_lowercase().contains(&query))
            .cloned()
            .collect::<Vec<_>>();
        filter_model.set_vec(rows);
    });

    window.run()
}
