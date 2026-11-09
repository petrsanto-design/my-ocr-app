//! Проверка моделей перед сборкой.
//!
//! Модели встраиваются в .exe через `include_bytes!`. Если файла нет, rustc выдал бы
//! малопонятную ошибку, поэтому сначала проверяем сами и объясняем, что делать.

use std::{env, fs, fs::File, io::Read, path::Path, process};

/// Требуемый файл: имя, минимальный разумный размер (байт), описание.
struct Required {
    file: &'static str,
    min_size: u64,
    what: &'static str,
}

// ВАЖНО: имена должны совпадать с src/models.rs.
const REQUIRED: [Required; 3] = [
    Required {
        file: "PP-OCRv5_mobile_det.mnn",
        min_size: 100_000,
        what: "детектор текста PP-OCRv5 (общий для всех языков)",
    },
    Required {
        file: "eslav_PP-OCRv5_mobile_rec_infer.mnn",
        min_size: 100_000,
        what: "распознаватель eslav (русский/украинский/белорусский + латиница)",
    },
    Required {
        file: "ppocr_keys_eslav.txt",
        min_size: 500,
        what: "словарь символов eslav (один символ на строку)",
    },
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR не задан");
    let dir = Path::new(&manifest).join("models");

    let mut problems: Vec<String> = Vec::new();

    for req in REQUIRED.iter() {
        let path = dir.join(req.file);
        println!("cargo:rerun-if-changed={}", path.display());

        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                problems.push(format!("  - НЕТ файла models/{}  ({})", req.file, req.what));
                continue;
            }
        };

        if meta.len() < req.min_size {
            problems.push(format!(
                "  - models/{} слишком маленький ({} байт, ожидалось не меньше {}). \
                 Возможно, файл повреждён или это заглушка.",
                req.file,
                meta.len(),
                req.min_size
            ));
            continue;
        }

        // Git LFS: вместо файла может лежать маленький текстовый «указатель».
        let mut head = [0u8; 64];
        let n = File::open(&path)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        if head[..n].starts_with(b"version https://git-lfs") {
            problems.push(format!(
                "  - models/{} — это указатель Git LFS, а не сам файл. Выполните `git lfs pull`.",
                req.file
            ));
            continue;
        }

        // Словарь: должен быть корректным UTF-8 и содержать сотни строк.
        if req.file.ends_with(".txt") {
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let lines = text.lines().count();
                    if lines < 300 {
                        problems.push(format!(
                            "  - models/{}: всего {} строк, ожидалось несколько сотен.",
                            req.file, lines
                        ));
                    }
                }
                Err(e) => problems.push(format!(
                    "  - models/{}: не читается как UTF-8 ({e}).",
                    req.file
                )),
            }
        }
    }

    if !problems.is_empty() {
        eprintln!();
        eprintln!("==================== ОШИБКА СБОРКИ: модели OCR ====================");
        eprintln!("Не найдены или некорректны файлы моделей в каталоге models/:");
        for p in &problems {
            eprintln!("{p}");
        }
        eprintln!();
        eprintln!("Как получить модели — см. раздел «Модели» в README.md");
        eprintln!("(готовые .mnn из репозитория ocr-rs либо конвертация скриптом");
        eprintln!("script/convert_paddle_to_mnn.py).");
        eprintln!("====================================================================");
        process::exit(1);
    }
}
