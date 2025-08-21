use criterion::{Criterion, criterion_group, criterion_main};
use document_parser::models::{DocumentFormat, ParserEngine};

fn document_parsing_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("document_parsing");

    group.bench_function("format_detection", |b| {
        b.iter(|| {
            let formats = vec![
                "test.pdf",
                "document.docx",
                "spreadsheet.xlsx",
                "presentation.pptx",
                "image.jpg",
                "audio.mp3",
            ];

            for file_path in formats {
                let _format =
                    DocumentFormat::from_extension(file_path.split('.').next_back().unwrap_or(""));
            }
        });
    });

    group.bench_function("engine_selection", |b| {
        b.iter(|| {
            let formats = vec![
                DocumentFormat::PDF,
                DocumentFormat::Word,
                DocumentFormat::Excel,
                DocumentFormat::PowerPoint,
                DocumentFormat::Image,
                DocumentFormat::Audio,
            ];

            for format in formats {
                let _engine = ParserEngine::select_for_format(&format);
            }
        });
    });

    group.finish();
}

criterion_group!(benches, document_parsing_benchmark);
criterion_main!(benches);
