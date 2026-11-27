//! Windows.Media.Ocr in-process (the pywinrt path of win_desktop.py), CJK-aware
//! text grouping, substring word-box targeting and Set-of-Mark drawing.

use crate::image::{Img, Labels};
use crate::py::{self, Error, Res};
use crate::state::{OcrItem, OcrWord};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

/// One word as the OCR engine reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWord {
    pub t: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub fn is_cjk_char(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303f}' | '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}' | '\u{ff00}'..='\u{ffef}')
}

/// Build a normalized OCR line entry with character-to-word span mapping.
///
/// WinRT OCR separates CJK characters as individual words with spaces between them.
/// This collapses spaces between adjacent CJK characters while preserving spaces
/// between Latin tokens, and records exact word spans so substring queries can
/// compute the tight bounding box of the matched text.
pub fn build_line_entry(raw_words: &[RawWord]) -> Option<OcrItem> {
    let words: Vec<OcrWord> = raw_words
        .iter()
        .filter_map(|w| {
            let t = py::strip(&w.t).to_string();
            (!t.is_empty()).then(|| OcrWord {
                t,
                x: py::round(w.x),
                y: py::round(w.y),
                w: py::round(w.w).max(1),
                h: py::round(w.h).max(1),
            })
        })
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut pos = 0usize;
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            let prev = &words[i - 1];
            let both_cjk = prev.t.chars().last().map(is_cjk_char).unwrap_or(false)
                && w.t.chars().next().map(is_cjk_char).unwrap_or(false);
            let mut gap_sep = !both_cjk;
            // Also add space if there is a wide visual gap between CJK columns
            let visual_gap = w.x - (prev.x + prev.w);
            let avg_h = prev.h.max(w.h).max(10);
            if !gap_sep && visual_gap as f64 > avg_h as f64 * 0.85 {
                gap_sep = true;
            }
            if gap_sep {
                text.push(' ');
                pos += 1;
            }
        }
        let start = pos;
        text.push_str(&w.t);
        pos += py::len(&w.t);
        spans.push((start, pos, i));
    }
    let text = py::strip(&text).to_string();
    if text.is_empty() {
        return None;
    }
    let left = words.iter().map(|w| w.x).min().unwrap_or(0);
    let top = words.iter().map(|w| w.y).min().unwrap_or(0);
    let right = words.iter().map(|w| w.x + w.w).max().unwrap_or(0);
    let bottom = words.iter().map(|w| w.y + w.h).max().unwrap_or(0);
    Some(OcrItem {
        text,
        x: left,
        y: top,
        w: (right - left).max(1),
        h: (bottom - top).max(1),
        cx: (left + right).div_euclid(2),
        cy: (top + bottom).div_euclid(2),
        words,
        spans,
    })
}

/// Convert raw OCR lines into sorted, normalized clickable items.
pub fn group_ocr_lines(raw_lines: &[Vec<RawWord>]) -> Vec<OcrItem> {
    let mut items: Vec<OcrItem> = raw_lines.iter().filter_map(|l| build_line_entry(l)).collect();
    // Sort top-to-bottom, then left-to-right (group by ~12px row bands)
    items.sort_by_key(|it| (it.y.div_euclid(12), it.x));
    items
}

/// Tight (x, y, w, h) for `query` matched inside `entry`, else the full line box.
pub fn find_substring_box(entry: &OcrItem, query: &str) -> (i64, i64, i64, i64) {
    let full = (entry.x, entry.y, entry.w, entry.h);
    let q = py::strip(query).to_lowercase();
    if q.is_empty() {
        return full;
    }
    let text_l = entry.text.to_lowercase();
    let Some(byte_idx) = text_l.find(&q) else { return full };
    let idx = text_l[..byte_idx].chars().count();
    let end = idx + py::len(&q);
    let matched: Vec<&OcrWord> =
        entry.spans.iter().filter(|(s, e, _)| *e > idx && *s < end).filter_map(|(_, _, i)| entry.words.get(*i)).collect();
    if matched.is_empty() {
        return full;
    }
    let left = matched.iter().map(|w| w.x).min().unwrap_or(0);
    let top = matched.iter().map(|w| w.y).min().unwrap_or(0);
    let right = matched.iter().map(|w| w.x + w.w).max().unwrap_or(0);
    let bottom = matched.iter().map(|w| w.y + w.h).max().unwrap_or(0);
    (left, top, (right - left).max(1), (bottom - top).max(1))
}

/// Draw numbered Set-of-Mark boxes [1], [2], ... onto a copy of `img`.
pub fn draw_som_overlay(img: &Img, items: &[(i64, &OcrItem)], scale: f64) -> Img {
    let mut out = img.clone();
    let labels = Labels::new();
    let inv = if scale != 0.0 { 1.0 / scale } else { 1.0 };
    let (iw, ih) = (out.w as i64, out.h as i64);
    for (idx, it) in items {
        let x1 = py::round(it.x as f64 * inv);
        let y1 = py::round(it.y as f64 * inv);
        let x2 = py::round((it.x + it.w) as f64 * inv);
        let y2 = py::round((it.y + it.h) as f64 * inv);
        out.outline_rect(x1, y1, x2, y2, (255, 64, 64, 220));
        let tag = idx.to_string();
        let tw = (7 * tag.len() as i64 + 4).max(14);
        let th = 13;
        let tx = x1.min(iw - tw - 1).max(0);
        let ty = if y1 >= th { y1 - th } else { y2.min(ih - th - 1) };
        out.fill_rect(tx, ty, tx + tw, ty + th, (220, 38, 38, 220));
        labels.draw(&mut out, tx + 2, ty + 1, &tag, (255, 255, 255, 255));
    }
    out
}

// ---------------------------------------------------------------- the engine

type Lines = Vec<Vec<RawWord>>;

struct Job {
    bgra: Vec<u8>,
    w: u32,
    h: u32,
    reply: mpsc::Sender<Result<(String, Lines), String>>,
}

struct Engine {
    engine: OcrEngine,
    lang: String,
}

fn text_of(e: windows::core::Error) -> String {
    let msg = e.message();
    if msg.is_empty() {
        format!("WinRT error 0x{:08X}", e.code().0 as u32)
    } else {
        msg.trim_end().to_string()
    }
}

fn make_engine() -> Result<Engine, String> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().ok().or_else(|| {
        let langs = OcrEngine::AvailableRecognizerLanguages().ok()?;
        if langs.Size().ok()? == 0 {
            return None;
        }
        OcrEngine::TryCreateFromLanguage(&langs.GetAt(0).ok()?).ok()
    });
    let Some(engine) = engine else { return Err("No Windows OCR language pack available".to_string()) };
    let lang = engine.RecognizerLanguage().and_then(|l| l.LanguageTag()).map(|t| t.to_string()).map_err(text_of)?;
    Ok(Engine { engine, lang })
}

fn recognize(slot: &mut Option<Engine>, job: &Job) -> Result<(String, Lines), String> {
    if slot.is_none() {
        *slot = Some(make_engine()?);
    }
    let eng = slot.as_ref().unwrap();
    let limit = OcrEngine::MaxImageDimension().ok().filter(|l| *l != 0).unwrap_or(10000);
    if job.w > limit || job.h > limit {
        return Err(format!("Image too large for Windows OCR ({}x{} > {limit})", job.w, job.h));
    }
    let run = || -> windows::core::Result<Lines> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(&job.bgra)?;
        let buffer = writer.DetachBuffer()?;
        let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            job.w as i32,
            job.h as i32,
            BitmapAlphaMode::Premultiplied,
        )?;
        let result = eng.engine.RecognizeAsync(&bitmap)?.get()?;
        let lines = result.Lines()?;
        let mut out = Vec::new();
        for i in 0..lines.Size()? {
            let words = lines.GetAt(i)?.Words()?;
            let mut row = Vec::new();
            for j in 0..words.Size()? {
                let word = words.GetAt(j)?;
                let r = word.BoundingRect()?;
                row.push(RawWord {
                    t: word.Text()?.to_string(),
                    x: py::round(r.X as f64) as f64,
                    y: py::round(r.Y as f64) as f64,
                    w: py::round(r.Width as f64) as f64,
                    h: py::round(r.Height as f64) as f64,
                });
            }
            out.push(row);
        }
        Ok(out)
    };
    run().map(|lines| (eng.lang.clone(), lines)).map_err(text_of)
}

/// WinRT calls run on one dedicated thread, so COM apartment state on the HTTP
/// and UI threads never matters, and a stuck call is abandoned after a timeout
/// instead of blocking the server.
static WORKER: Mutex<Option<mpsc::Sender<Job>>> = Mutex::new(None);

fn start_worker() -> mpsc::Sender<Job> {
    let (tx, rx) = mpsc::channel::<Job>();
    let _ = std::thread::Builder::new().name("gf-ocr".to_string()).spawn(move || {
        let mut engine: Option<Engine> = None;
        for job in rx {
            let result = recognize(&mut engine, &job);
            let _ = job.reply.send(result);
        }
    });
    tx
}

/// Run Windows built-in OCR on an image and return (lang_tag, grouped_items).
pub fn ocr_image(img: &Img, timeout: f64) -> Res<(String, Vec<OcrItem>)> {
    // holding the lock for the whole call serializes OCR, as the Python version does
    let mut worker = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    let (reply, answer) = mpsc::channel();
    let job = Job { bgra: img.to_bgra(), w: img.w, h: img.h, reply };
    let sender = worker.get_or_insert_with(start_worker);
    let job = match sender.send(job) {
        Ok(()) => None,
        Err(mpsc::SendError(job)) => Some(job),
    };
    if let Some(job) = job {
        // the thread is gone; start a fresh one
        let fresh = start_worker();
        let _ = fresh.send(job);
        *worker = Some(fresh);
    }
    match answer.recv_timeout(Duration::from_secs_f64(timeout)) {
        Ok(Ok((lang, lines))) => Ok((lang, group_ocr_lines(&lines))),
        Ok(Err(msg)) => Err(Error::runtime(msg)),
        Err(_) => {
            // Abandon the stuck thread; the next call starts a fresh one.
            *worker = None;
            Err(Error::runtime("Windows OCR timed out"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(t: &str, x: f64, y: f64, w: f64, h: f64) -> RawWord {
        RawWord { t: t.to_string(), x, y, w, h }
    }

    #[test]
    fn cjk_grouping_column_gap_and_substring_box() {
        // adjacent CJK characters collapse, Latin tokens keep their space
        let line = vec![
            word("確", 10.0, 10.0, 20.0, 20.0),
            word("認", 32.0, 10.0, 20.0, 20.0),
            word("OK", 60.0, 10.0, 30.0, 20.0),
            word("now", 95.0, 10.0, 40.0, 20.0),
        ];
        let entry = build_line_entry(&line).unwrap();
        assert_eq!(entry.text, "確認 OK now");
        assert_eq!((entry.x, entry.y, entry.w, entry.h), (10, 10, 125, 20));
        assert_eq!((entry.cx, entry.cy), (72, 20));
        // a wide gap between CJK characters is a column break
        let cols = vec![word("左", 10.0, 10.0, 20.0, 20.0), word("右", 100.0, 10.0, 20.0, 20.0)];
        assert_eq!(build_line_entry(&cols).unwrap().text, "左 右");
        // substring -> tight box over the matched words only
        assert_eq!(find_substring_box(&entry, "ok"), (60, 10, 30, 20));
        assert_eq!(find_substring_box(&entry, "確認"), (10, 10, 42, 20));
        assert_eq!(find_substring_box(&entry, "認 OK"), (32, 10, 58, 20));
        assert_eq!(find_substring_box(&entry, "missing"), (10, 10, 125, 20));
        assert_eq!(find_substring_box(&entry, "  "), (10, 10, 125, 20));
        // blank words are dropped, empty lines vanish
        assert!(build_line_entry(&[word("  ", 0.0, 0.0, 5.0, 5.0)]).is_none());
        // rows sort top to bottom in 12 px bands, then left to right
        let items = group_ocr_lines(&[
            vec![word("second", 200.0, 14.0, 50.0, 12.0)],
            vec![word("third", 10.0, 40.0, 50.0, 12.0)],
            vec![word("first", 10.0, 20.0, 50.0, 12.0)],
        ]);
        let order: Vec<&str> = items.iter().map(|it| it.text.as_str()).collect();
        assert_eq!(order, vec!["first", "second", "third"]);
    }

    #[test]
    fn draw_som_overlay_does_not_mutate_source_image() {
        let mut src = Img::new(120, 80);
        for b in src.rgb.iter_mut() {
            *b = 255;
        }
        let before = src.clone();
        let entry = build_line_entry(&[word("Hello", 20.0, 30.0, 60.0, 16.0)]).unwrap();
        let marked = draw_som_overlay(&src, &[(1, &entry)], 1.0);
        assert_eq!(src, before);
        assert_ne!(marked, src);
        assert_eq!((marked.w, marked.h), (120, 80));
        // the outline is where the box is
        let px = |img: &Img, x: usize, y: usize| img.rgb[(y * 120 + x) * 3..(y * 120 + x) * 3 + 3].to_vec();
        assert_ne!(px(&marked, 20, 30), vec![255, 255, 255]);
        assert_eq!(px(&marked, 50, 38), vec![255, 255, 255]);
    }
}
