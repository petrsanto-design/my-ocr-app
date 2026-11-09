//! Фоновый поток OCR: GUI отправляет задания в канал и получает события обратно.
//!
//! Движок (`OcrEngine`) создаётся лениво внутри рабочего потока при первом
//! распознавании, поэтому окно появляется мгновенно, а загрузка моделей идёт в фоне.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{unbounded, Receiver, Sender};
use image::DynamicImage;
use ocr_rs::{OcrEngine, OcrEngineConfig};

use crate::models;

/// Язык распознавания (пункты выпадающего списка).
///
/// ВАЖНО: в v0.1 все три режима используют ОДНУ модель eslav (русский + латиница),
/// отдельной английской модели нет. Выбор языка сохранён в интерфейсе и передаётся
/// воркеру, чтобы позже подключить отдельные модели (см. `model_for`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Russian,
    English,
    Both,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Russian, Lang::English, Lang::Both];

    pub fn label(self) -> &'static str {
        match self {
            Lang::Russian => "Русский",
            Lang::English => "English",
            Lang::Both => "Оба",
        }
    }
}

/// Идентификатор набора моделей. Сейчас он один; enum нужен как точка расширения.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ModelSet {
    Eslav,
}

/// Какой набор моделей использовать для выбранного языка.
fn model_for(_lang: Lang) -> ModelSet {
    // Все режимы → eslav (он распознаёт и кириллицу, и латиницу).
    ModelSet::Eslav
}

/// Задание для воркера.
pub enum Job {
    Recognize {
        image: Arc<DynamicImage>,
        lang: Lang,
    },
}

/// События от воркера к GUI.
pub enum Event {
    /// Текстовый статус («Загрузка моделей…», «Распознавание…»).
    Status(String),
    /// Результат: распознанный текст либо сообщение об ошибке.
    Finished(Result<String, String>),
}

/// Каналы связи с воркером.
pub struct WorkerHandle {
    pub jobs: Sender<Job>,
    pub events: Receiver<Event>,
}

/// Запускает рабочий поток. `ctx` нужен, чтобы будить GUI после каждого события.
pub fn spawn(ctx: egui::Context) -> WorkerHandle {
    let (job_tx, job_rx) = unbounded::<Job>();
    let (ev_tx, ev_rx) = unbounded::<Event>();

    thread::Builder::new()
        .name("ocr-worker".into())
        // Запас стека: нативный инференс и обработка больших изображений.
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let mut engine: Option<(ModelSet, OcrEngine)> = None;

            // Цикл завершится сам, когда GUI закроется и канал заданий закроется.
            while let Ok(job) = job_rx.recv() {
                match job {
                    Job::Recognize { image, lang } => {
                        let send_status = |s: &str| {
                            let _ = ev_tx.send(Event::Status(s.to_string()));
                            ctx.request_repaint();
                        };

                        // Паника внутри нативной обёртки не должна убить воркер.
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            run_job(&mut engine, &image, lang, &send_status)
                        }));

                        let result = match outcome {
                            Ok(r) => r,
                            Err(_) => {
                                // Состояние движка после паники считаем испорченным.
                                engine = None;
                                Err("Внутренняя ошибка при распознавании (паника). \
                                     Попробуйте ещё раз или другое изображение."
                                    .to_string())
                            }
                        };

                        let _ = ev_tx.send(Event::Finished(result));
                        ctx.request_repaint();
                    }
                }
            }
        })
        .expect("не удалось запустить поток OCR");

    WorkerHandle {
        jobs: job_tx,
        events: ev_rx,
    }
}

/// Выполняет одно распознавание. Создаёт движок, если его ещё нет.
fn run_job(
    engine: &mut Option<(ModelSet, OcrEngine)>,
    image: &DynamicImage,
    lang: Lang,
    status: &dyn Fn(&str),
) -> Result<String, String> {
    let wanted = model_for(lang);

    // Пересоздаём движок, только если нужен другой набор моделей.
    let need_load = match engine {
        Some((set, _)) => *set != wanted,
        None => true,
    };

    if need_load {
        status("Загрузка моделей…");
        let threads = thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .clamp(1, 8);

        // Порог уверенности снижен с 0.5 (по умолчанию), чтобы не терять слабый текст.
        let config = OcrEngineConfig::new()
            .with_threads(threads)
            .with_min_result_confidence(0.3);

        let (det, rec, charset) = match wanted {
            ModelSet::Eslav => (
                models::DET_MODEL,
                models::REC_MODEL_ESLAV,
                models::CHARSET_ESLAV,
            ),
        };

        let eng = OcrEngine::from_bytes(det, rec, charset, Some(config))
            .map_err(|e| format!("Не удалось загрузить модели OCR: {e}"))?;
        *engine = Some((wanted, eng));
    }

    status("Распознавание…");
    let (_, eng) = engine.as_ref().expect("движок создан выше");

    let results = eng
        .recognize(image)
        .map_err(|e| format!("Ошибка распознавания: {e}"))?;

    // Копируем только нужное: текст и геометрию прямоугольника.
    let lines: Vec<Line> = results
        .iter()
        .filter_map(|r| {
            let text = r.text.trim();
            if text.is_empty() {
                return None;
            }
            Some(Line {
                text: text.to_string(),
                left: r.bbox.rect.left(),
                top: r.bbox.rect.top(),
                height: r.bbox.rect.height() as i32,
            })
        })
        .collect();

    Ok(assemble_text(lines))
}

/// Фрагмент распознанного текста с положением на изображении.
struct Line {
    text: String,
    left: i32,
    top: i32,
    height: i32,
}

/// Собирает фрагменты в текст в порядке чтения: сверху вниз, слева направо.
/// Фрагменты на одной горизонтали склеиваются пробелом.
///
/// Ограничение: многоколоночную вёрстку этот алгоритм не разбирает —
/// колонки будут перемешаны по строкам.
fn assemble_text(mut items: Vec<Line>) -> String {
    items.sort_by_key(|l| (l.top, l.left));

    let mut rows: Vec<Vec<Line>> = Vec::new();
    for item in items {
        let same_row = rows
            .last()
            .map(|row| on_same_row(&row[0], &item))
            .unwrap_or(false);
        if same_row {
            rows.last_mut().expect("строка есть").push(item);
        } else {
            rows.push(vec![item]);
        }
    }

    let mut out_rows: Vec<String> = Vec::with_capacity(rows.len());
    for mut row in rows {
        row.sort_by_key(|l| l.left);
        let joined = row
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        out_rows.push(joined);
    }
    out_rows.join("\n")
}

/// Лежат ли два фрагмента на одной строке: центры по вертикали
/// отличаются не более чем на половину меньшей высоты.
fn on_same_row(a: &Line, b: &Line) -> bool {
    let ca = a.top + a.height / 2;
    let cb = b.top + b.height / 2;
    let min_h = a.height.min(b.height).max(1);
    (ca - cb).abs() * 2 <= min_h
}
