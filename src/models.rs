//! Модели OCR, встроенные в .exe. Никаких скачиваний и чтения файлов в рантайме.
//!
//! Имена файлов должны совпадать с build.rs (там же проверяется их наличие).
//! Пути в `include_bytes!` — относительно этого файла (src/models.rs).

/// Детектор текста PP-OCRv5 mobile (MNN). Общий для всех языков.
pub static DET_MODEL: &[u8] = include_bytes!("../models/PP-OCRv5_mobile_det.mnn");

/// Распознаватель PP-OCRv5 eslav (русский, украинский, белорусский + латиница, цифры).
pub static REC_MODEL_ESLAV: &[u8] =
    include_bytes!("../models/eslav_PP-OCRv5_mobile_rec_infer.mnn");

/// Словарь символов для eslav (UTF-8, один символ на строку).
/// Служебные токены (blank/padding) добавляет сама библиотека ocr-rs.
pub static CHARSET_ESLAV: &[u8] = include_bytes!("../models/ppocr_keys_eslav.txt");
