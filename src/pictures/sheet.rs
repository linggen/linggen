//! A contact sheet: several pictures tiled into one small JPEG, each tile
//! labelled by its number, so a model that sees can judge a search's
//! pictures in one look (`ImageSearch`, doc/chat-spec.md § Sources and
//! pictures in answers). Pure: pictures in, bytes out.

use image::{imageops, DynamicImage, Rgb, RgbImage};

/// Tiles a row; the sheet is at most `COLS * TILE_W + (COLS + 1) * GAP`
/// (1020) px wide.
const COLS: u32 = 4;
const TILE_W: u32 = 250;
const TILE_H: u32 = 188;
const GAP: u32 = 4;
const QUALITY: u8 = 80;
/// One glyph pixel drawn as this many.
const SCALE: u32 = 5;
const PAD: u32 = 4;

const BACKGROUND: Rgb<u8> = Rgb([90, 90, 90]);
const TILE: Rgb<u8> = Rgb([238, 238, 238]);
const LABEL: Rgb<u8> = Rgb([255, 214, 0]);
const INK: Rgb<u8> = Rgb([0, 0, 0]);

/// Digits 0–9, five rows of three bits each.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// The sheet of `tiles` (label, picture) as JPEG; `None` when empty.
pub fn compose(tiles: &[(u32, DynamicImage)]) -> Option<Vec<u8>> {
    if tiles.is_empty() {
        return None;
    }
    let n = tiles.len() as u32;
    let cols = n.min(COLS);
    let rows = n.div_ceil(COLS);
    let mut sheet = RgbImage::from_pixel(
        cols * TILE_W + (cols + 1) * GAP,
        rows * TILE_H + (rows + 1) * GAP,
        BACKGROUND,
    );
    for (i, (label, picture)) in tiles.iter().enumerate() {
        let i = i as u32;
        let x = GAP + (i % COLS) * (TILE_W + GAP);
        let y = GAP + (i / COLS) * (TILE_H + GAP);
        draw_tile(&mut sheet, x, y, picture);
        draw_label(&mut sheet, x, y, *label);
    }
    jpeg(sheet)
}

/// The picture fitted into its tile, centred, its shape kept.
fn draw_tile(sheet: &mut RgbImage, x: u32, y: u32, picture: &DynamicImage) {
    let tile = RgbImage::from_pixel(TILE_W, TILE_H, TILE);
    imageops::replace(sheet, &tile, x as i64, y as i64);
    let fitted = picture
        .resize(TILE_W, TILE_H, imageops::FilterType::Triangle)
        .to_rgb8();
    let dx = (TILE_W - fitted.width()) / 2;
    let dy = (TILE_H - fitted.height()) / 2;
    imageops::replace(sheet, &fitted, (x + dx) as i64, (y + dy) as i64);
}

/// The number in black on a yellow box at the tile's top left.
fn draw_label(sheet: &mut RgbImage, x: u32, y: u32, label: u32) {
    let digits: Vec<usize> = label
        .to_string()
        .bytes()
        .map(|b| (b - b'0') as usize)
        .collect();
    let advance = 4 * SCALE;
    let w = digits.len() as u32 * advance - SCALE + 2 * PAD;
    let h = 5 * SCALE + 2 * PAD;
    fill(sheet, x, y, w, h, LABEL);
    for (k, d) in digits.iter().enumerate() {
        draw_digit(sheet, x + PAD + k as u32 * advance, y + PAD, *d);
    }
}

fn draw_digit(sheet: &mut RgbImage, x: u32, y: u32, digit: usize) {
    for (row, bits) in DIGITS[digit].iter().enumerate() {
        for col in 0..3u32 {
            if bits & (0b100 >> col) != 0 {
                let (px, py) = (x + col * SCALE, y + row as u32 * SCALE);
                fill(sheet, px, py, SCALE, SCALE, INK);
            }
        }
    }
}

fn fill(sheet: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
    for py in y..(y + h).min(sheet.height()) {
        for px in x..(x + w).min(sheet.width()) {
            sheet.put_pixel(px, py, color);
        }
    }
}

fn jpeg(sheet: RgbImage) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY);
    DynamicImage::ImageRgb8(sheet)
        .write_with_encoder(encoder)
        .ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(w, h, Rgb([20, 120, 200])))
    }

    #[test]
    fn ten_pictures_make_one_small_sheet_of_three_rows() {
        let tiles: Vec<(u32, DynamicImage)> =
            (1..=10).map(|i| (i, picture(1600 + i * 10, 900))).collect();
        let bytes = compose(&tiles).expect("a sheet");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG");
        let sheet = image::load_from_memory(&bytes).unwrap();
        assert!(sheet.width() <= 1024, "{}", sheet.width());
        assert_eq!(sheet.width(), 4 * TILE_W + 5 * GAP);
        assert_eq!(sheet.height(), 3 * TILE_H + 4 * GAP);
    }

    #[test]
    fn each_tile_carries_its_label_and_none_means_no_sheet() {
        let bytes = compose(&[(7, picture(300, 300)), (12, picture(50, 400))]).unwrap();
        let sheet = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(sheet.width(), 2 * TILE_W + 3 * GAP);
        // The label's yellow box sits at each tile's top left corner.
        let yellow = |x: u32, y: u32| {
            let p = sheet.get_pixel(x, y);
            p[0] > 200 && p[1] > 170 && p[2] < 80
        };
        assert!(yellow(GAP + 1, GAP + 1));
        assert!(yellow(2 * GAP + TILE_W + 1, GAP + 1));
        assert!(compose(&[]).is_none());
    }
}
