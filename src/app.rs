//! GUI приложения (eframe/egui 0.37).
//!
//! В egui 0.37 трейт `eframe::App` реализуется через `ui(&mut self, ui, frame)`,
//! а панели показываются через `egui::Panel::{top,bottom}(..).show(ui, ..)` и
//! `egui::CentralPanel::default().show(ui, ..)`.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use image::DynamicImage;

use crate::worker::{self, Event, Job, Lang, WorkerHandle};

/// Максимальная сторона изображения для текстуры предпросмотра (пиксели).
/// На OCR не влияет: в движок уходит оригинал.
const PREVIEW_MAX_SIDE: u32 = 2048;

pub struct OcrApp {
    worker: WorkerHandle,

    /// Исходное изображение (для OCR). Arc — чтобы дёшево передавать в воркер.
    image: Option<Arc<DynamicImage>>,
    image_path: Option<PathBuf>,
    texture: Option<egui::TextureHandle>,

    lang: Lang,
    /// Редактируемый распознанный текст.
    text: String,

    /// Идёт ли распознавание.
    busy: bool,
    status: String,
    error: Option<String>,
}

impl OcrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            worker: worker::spawn(cc.egui_ctx.clone()),
            image: None,
            image_path: None,
            texture: None,
            lang: Lang::Russian,
            text: String::new(),
            busy: false,
            status: "Откройте изображение (PNG, JPG, TIFF, BMP)".to_string(),
            error: None,
        }
    }

    /// Забирает все накопившиеся события воркера.
    fn poll_worker(&mut self) {
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::Status(s) => self.status = s,
                Event::Finished(Ok(text)) => {
                    self.busy = false;
                    self.error = None;
                    if text.trim().is_empty() {
                        self.status = "Текст на изображении не найден".to_string();
                        // Не оставляем результат предыдущего файла.
                        self.text.clear();
                    } else {
                        self.status = "Готово".to_string();
                        self.text = text;
                    }
                }
                Event::Finished(Err(msg)) => {
                    self.busy = false;
                    self.status = "Ошибка".to_string();
                    self.error = Some(msg);
                }
            }
        }
    }

    /// Диалог выбора файла.
    fn pick_file(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Изображения", &["png", "jpg", "jpeg", "tif", "tiff", "bmp"])
            .pick_file();
        if let Some(path) = picked {
            self.load_image(ctx, path);
        }
    }

    /// Декодирует изображение и готовит текстуру предпросмотра.
    /// (Декодирование идёт в потоке GUI: для обычных файлов это доли секунды.)
    fn load_image(&mut self, ctx: &egui::Context, path: PathBuf) {
        match image::open(&path) {
            Ok(img) => {
                // Прозрачность → белый фон (иначе прозрачные пиксели читаются как чёрные).
                let img = flatten_alpha(img);
                let preview = make_preview(&img);
                self.texture = Some(ctx.load_texture(
                    "preview",
                    preview,
                    egui::TextureOptions::LINEAR,
                ));
                self.status = format!(
                    "Загружено: {} ({}×{})",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    img.width(),
                    img.height()
                );
                self.image = Some(Arc::new(img));
                self.image_path = Some(path);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Не удалось открыть файл: {e}"));
                self.status = "Ошибка".to_string();
            }
        }
    }

    /// Отправляет текущее изображение в воркер.
    fn start_ocr(&mut self) {
        let Some(img) = self.image.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        self.status = "В очереди…".to_string();
        let sent = self.worker.jobs.send(Job::Recognize {
            image: img,
            lang: self.lang,
        });
        if sent.is_err() {
            self.busy = false;
            self.error = Some("Поток OCR недоступен (завершился аварийно).".to_string());
        }
    }

    /// Копирование распознанного текста в системный буфер обмена.
    fn copy_to_clipboard(&mut self) {
        let result = arboard::Clipboard::new().and_then(|mut cb| cb.set_text(self.text.clone()));
        match result {
            Ok(()) => {
                self.error = None;
                self.status = "Текст скопирован в буфер обмена".to_string();
            }
            Err(e) => self.error = Some(format!("Не удалось скопировать: {e}")),
        }
    }

    /// Сохранение текста в TXT (UTF-8 с BOM — чтобы Блокнот Windows корректно показал кириллицу).
    fn save_txt(&mut self) {
        let suggested = self
            .image_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}.txt", s.to_string_lossy()))
            .unwrap_or_else(|| "recognized.txt".to_string());

        let Some(path) = rfd::FileDialog::new()
            .add_filter("Текстовый файл", &["txt"])
            .set_file_name(suggested)
            .save_file()
        else {
            return;
        };

        let mut bytes: Vec<u8> = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        bytes.extend_from_slice(self.text.as_bytes());

        match std::fs::write(&path, bytes) {
            Ok(()) => {
                self.error = None;
                self.status = format!("Сохранено: {}", path.display());
            }
            Err(e) => self.error = Some(format!("Не удалось сохранить файл: {e}")),
        }
    }

    // ---------------------------------------------------------------- UI

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new("Открыть файл"))
                .clicked()
            {
                self.pick_file(ctx);
            }

            ui.add_enabled_ui(!self.busy, |ui| {
                egui::ComboBox::from_label("Язык")
                    .selected_text(self.lang.label())
                    .show_ui(ui, |ui| {
                        for lang in Lang::ALL {
                            ui.selectable_value(&mut self.lang, lang, lang.label());
                        }
                    });
            });

            let can_run = self.image.is_some() && !self.busy;
            if ui
                .add_enabled(can_run, egui::Button::new("Распознать"))
                .clicked()
            {
                self.start_ocr();
            }

            if self.busy {
                ui.spinner();
                ui.label("Обработка...");
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
            } else {
                ui.label(&self.status);
            }
        });
        ui.label(
            egui::RichText::new(
                "Модель: PP-OCRv5 eslav (рус/укр/бел + латиница); \
                 все режимы языка используют одну модель",
            )
            .small()
            .weak(),
        );
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            // --- Левая колонка: предпросмотр
            cols[0].heading("Изображение");
            match &self.texture {
                Some(tex) => {
                    cols[0].add(egui::Image::new(tex).shrink_to_fit());
                }
                None => {
                    cols[0].label("Файл не выбран");
                }
            }

            // --- Правая колонка: текст и кнопки
            cols[1].heading("Распознанный текст");
            let text_height = (cols[1].available_height() - 44.0).max(120.0);
            egui::ScrollArea::vertical()
                .max_height(text_height)
                .auto_shrink([false, false])
                .show(&mut cols[1], |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.text)
                            .desired_width(f32::INFINITY)
                            .desired_rows(18),
                    );
                });

            cols[1].horizontal(|ui| {
                let has_text = !self.text.is_empty();
                if ui
                    .add_enabled(has_text, egui::Button::new("Копировать в буфер"))
                    .clicked()
                {
                    self.copy_to_clipboard();
                }
                if ui
                    .add_enabled(has_text, egui::Button::new("Сохранить как TXT"))
                    .clicked()
                {
                    self.save_txt();
                }
            });
        });
    }
}

impl eframe::App for OcrApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker();
        let ctx = ui.ctx().clone();

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui, &ctx);
            ui.add_space(4.0);
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(2.0);
            self.status_bar(ui);
        });

        // CentralPanel всегда добавляется последним.
        egui::CentralPanel::default().show(ui, |ui| {
            self.content(ui);
        });
    }
}

/// Готовит изображение для текстуры предпросмотра (уменьшает очень большие).
fn make_preview(img: &DynamicImage) -> egui::ColorImage {
    let rgba = if img.width() > PREVIEW_MAX_SIDE || img.height() > PREVIEW_MAX_SIDE {
        img.resize(
            PREVIEW_MAX_SIDE,
            PREVIEW_MAX_SIDE,
            image::imageops::FilterType::Triangle,
        )
        .to_rgba8()
    } else {
        img.to_rgba8()
    };
    let size = [rgba.width() as usize, rgba.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
}

/// Накладывает изображение с прозрачностью на белый фон.
fn flatten_alpha(img: DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return img;
    }
    let rgba = img.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as u32;
        let blend = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([blend(p[0]), blend(p[1]), blend(p[2])]));
    }
    DynamicImage::ImageRgb8(rgb)
}
