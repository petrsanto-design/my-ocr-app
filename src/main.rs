//! OCR-приложение: eframe/egui + ocr-rs (PaddleOCR PP-OCRv5 eslav через MNN).
//! Один .exe, модели встроены, интернет в рантайме не нужен.

// В release-сборке не показываем консольное окно.
// (Побочный эффект: stderr и сообщения паник не видны.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod models;
mod worker;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OCR — распознавание текста")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };

    let result = eframe::run_native(
        "ocr_app",
        options,
        Box::new(|cc| Ok(Box::new(app::OcrApp::new(cc)))),
    );

    // В release-сборке консоли нет, поэтому ошибку запуска (например, не удалось
    // создать графический контекст) показываем окном, а не молча закрываемся.
    if let Err(e) = &result {
        let _ = rfd::MessageDialog::new()
            .set_title("OCR: не удалось запустить окно")
            .set_description(format!("{e}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    result
}
