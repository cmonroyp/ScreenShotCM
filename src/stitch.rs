//! Joins the frames of a scrolling capture into one tall image. Each frame is
//! the same area of the screen, read again after the content under it moved
//! up; the new frame's rows are matched against the previous one to find how
//! far it moved, and only the rows that appeared at the bottom are added.
//!
//! Rows are compared by a profile: the mean brightness of 32 slices across
//! the row. Not their exact pixels: a browser at 125 % scrolls by fractions
//! of a pixel and draws each line's anti-aliasing a little differently at
//! every position, so no row ever comes back identical (Edge lost 20 reads of
//! 30 that way). The offset kept is the one whose rows differ least, and only
//! when it is clearly better than any other.
//!
//! A band that stays put at the top or the bottom of the area (a sticky
//! header, a toolbar, a message box) is kept once rather than repeated, and
//! the right edge — where a scroll bar moves on every frame — is left out.
//! Rows of a single colour match anything, so they are not counted as
//! evidence.

use crate::image::{Image, luma};

/// What one frame added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The content did not move.
    Same,
    /// This many new rows were added.
    Added(usize),
    /// The frame does not continue the previous one: scrolled too far at
    /// once, scrolled up, or something changed in place. Nothing was added.
    Lost,
    /// The image reached its height limit; nothing more will be added.
    Full,
}

pub struct Stitcher {
    width: usize,
    height: usize,
    /// Columns compared: from the left edge to before the scroll bar.
    cols: usize,
    previous: Vec<Profile>,
    pixels: Vec<u8>,
    rows: usize,
    max_rows: usize,
    /// The last distance moved, to choose between equally good matches in
    /// repetitive content (a list of identical rows).
    last_dy: usize,
    full: bool,
    /// The pixels of the frame `previous` describes.
    previous_frame: Vec<u8>,
    /// Per column: how many matched frames showed it standing still while
    /// the content moved — a window's border, rounded corner or scroll bar
    /// inside the chosen area, which would otherwise repeat down the image.
    chrome_votes: Vec<u32>,
    matched: u32,
}

impl Stitcher {
    /// Starts from the first frame. `max_rows` bounds the result's height.
    pub fn new(first: &Image, max_rows: usize) -> Self {
        let width = first.width as usize;
        let height = first.height as usize;
        // A scroll bar is at most ~20 px at 100 %; at least 90 % of the width
        // is kept even in a narrow area.
        let margin = (width / 10).min(32);
        let cols = (width - margin).max(1);
        let max_rows = max_rows.max(height);
        Self {
            width,
            height,
            cols,
            previous: profiles(first, cols),
            pixels: first.pixels.clone(),
            rows: height,
            max_rows,
            last_dy: 0,
            full: false,
            previous_frame: first.pixels.clone(),
            chrome_votes: vec![0; width],
            matched: 0,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    /// The image so far, without the window's own edges if the area took
    /// them in.
    pub fn image(&self) -> Image {
        let (left, right) = self.content_columns();
        if (left, right) == (0, self.width) {
            return Image {
                width: self.width as u32,
                height: self.rows as u32,
                pixels: self.pixels.clone(),
            };
        }
        let stride = self.width * 4;
        let mut pixels = Vec::with_capacity((right - left) * 4 * self.rows);
        for row in self.pixels.chunks_exact(stride) {
            pixels.extend_from_slice(&row[left * 4..right * 4]);
        }
        Image {
            width: (right - left) as u32,
            height: self.rows as u32,
            pixels,
        }
    }

    /// The columns that moved with the content: everything between the
    /// innermost edge column that stood still in at least half the matched
    /// frames, on either side.
    fn content_columns(&self) -> (usize, usize) {
        if self.matched < 2 {
            return (0, self.width);
        }
        let need = self.matched.div_ceil(2);
        let zone = edge_zone(self.width);
        let chrome = |x: usize| self.chrome_votes[x] >= need;
        let left = (0..zone).rev().find(|&x| chrome(x)).map_or(0, |x| x + 1);
        let right = (self.width - zone..self.width)
            .find(|&x| chrome(x))
            .unwrap_or(self.width);
        (left, right)
    }

    /// Counts, for each edge column, whether it stood still while the
    /// content moved `dy` rows up: as a whole it differs from itself in
    /// place far less than from where the content went. A single pixel is no
    /// evidence — a white background stays white wherever the text went.
    fn vote_chrome(&mut self, frame: &Image, top: usize, end: usize, dy: usize) {
        let stride = self.width * 4;
        let zone = edge_zone(self.width);
        let at = |p: &[u8], i: usize| luma(&p[i..i + 3]) as i32;
        let (now, before) = (&frame.pixels, &self.previous_frame);
        for x in (0..zone).chain(self.width - zone..self.width) {
            let (mut in_place, mut with_content) = (0u64, 0u64);
            for y in top..end {
                if y < top + dy && y + dy >= self.height {
                    continue;
                }
                let i = y * stride + x * 4;
                // Where this row's content went (or came from) in the other frame.
                let moved = if y >= top + dy {
                    at(before, i) - at(now, i - dy * stride)
                } else {
                    at(now, i) - at(before, i + dy * stride)
                };
                in_place += (at(now, i) - at(before, i)).unsigned_abs() as u64;
                with_content += moved.unsigned_abs() as u64;
            }
            // Something to see (a corner, an arrow, a thumb), and it stayed.
            if with_content > 2 * in_place + 400 {
                self.chrome_votes[x] += 1;
            }
        }
        self.matched += 1;
    }

    /// The last `rows` rows of the image so far, for a preview.
    pub fn tail(&self, rows: usize) -> Image {
        let rows = rows.min(self.rows).max(1);
        let start = (self.rows - rows) * self.width * 4;
        Image {
            width: self.width as u32,
            height: rows as u32,
            pixels: self.pixels[start..].to_vec(),
        }
    }

    pub fn push(&mut self, frame: &Image) -> Step {
        if self.full {
            return Step::Full;
        }
        // A few rows cannot show a movement, and the band arithmetic below
        // needs room for its bottom margin (scroll.rs refuses such areas too).
        if frame.width as usize != self.width
            || frame.height as usize != self.height
            || self.height < MIN_HEIGHT
        {
            return Step::Lost;
        }
        let next = profiles(frame, self.cols);
        let h = self.height;
        if next == self.previous {
            return Step::Same;
        }
        // Bands that did not move, from the top and from the bottom.
        let cap = h / 3;
        let still = |i: usize| distance(&next[i], &self.previous[i]) <= STILL;
        let top = (0..cap).take_while(|&i| still(i)).count();
        // New rows are never taken from the very bottom of the area: a
        // window's lower border and rounded corners sit there, unmoved over
        // a row that otherwise moves, and would repeat down the image. Those
        // rows go on once, at the end, from the last frame.
        let bottom = (0..cap)
            .take_while(|&i| still(h - 1 - i))
            .count()
            .max((h / 40).clamp(4, 16));
        let dy = match offset(&self.previous, &next, top, bottom, self.last_dy) {
            Some(0) => return Step::Same,
            Some(dy) => dy,
            None => return Step::Lost,
        };
        let end = h - bottom;
        self.vote_chrome(frame, top, end, dy);
        // The previous frame's bottom band ends the image; the new frame has
        // the same band, so it goes back on after the new rows.
        let stride = self.width * 4;
        let room = self.max_rows.saturating_sub(self.rows);
        let add = dy.min(room);
        self.pixels.truncate((self.rows - bottom) * stride);
        self.pixels
            .extend_from_slice(&frame.pixels[(end - dy) * stride..(end - dy + add) * stride]);
        self.pixels
            .extend_from_slice(&frame.pixels[end * stride..h * stride]);
        self.rows += add;
        self.previous = next;
        self.previous_frame.clear();
        self.previous_frame.extend_from_slice(&frame.pixels);
        self.last_dy = dy;
        if add < dy {
            self.full = true;
            return Step::Full;
        }
        Step::Added(add)
    }
}

/// How far in from each side a window's edge can be: 12 % of the width, at
/// most 60 px (a scroll bar is about 17 px at 100 %, 34 at 200 %).
fn edge_zone(width: usize) -> usize {
    (width * 12 / 100).min(60)
}

/// The shortest frame worth comparing: the bottom margin (up to 16 rows)
/// and enough moving rows to match.
const MIN_HEIGHT: usize = 48;

/// The brightness of a BGR(A) pixel: Rec. 601 weights, in 256ths.
const SLICES: usize = 32;
type Profile = [u8; SLICES];

/// Two rows this close (summed over the slices) did not move: a fixed band.
const STILL: u32 = 8;
/// The most a matching row may differ on average, per slice: room for the
/// anti-aliasing of a line drawn a fraction of a pixel lower.
const MATCH_PER_SLICE: f32 = 10.0;
/// The best offset must beat every other (away from its neighbours) by this
/// factor, or the content is too uniform to tell.
const CLEAR_WIN: f32 = 0.6;

/// One profile per row: mean brightness of `SLICES` slices of its first
/// `cols` pixels.
fn profiles(image: &Image, cols: usize) -> Vec<Profile> {
    let stride = image.width as usize * 4;
    image
        .pixels
        .chunks_exact(stride)
        .map(|row| {
            let mut p = [0u8; SLICES];
            for (i, slot) in p.iter_mut().enumerate() {
                let (a, b) = (
                    i * cols / SLICES,
                    ((i + 1) * cols / SLICES).max(i * cols / SLICES + 1),
                );
                let mut sum = 0u32;
                for px in row[a * 4..b * 4].as_chunks::<4>().0 {
                    sum += luma(px);
                }
                *slot = (sum / (b - a) as u32) as u8;
            }
            p
        })
        .collect()
}

fn distance(a: &Profile, b: &Profile) -> u32 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u32).sum()
}

fn is_plain(p: &Profile) -> bool {
    let (lo, hi) = p
        .iter()
        .fold((u8::MAX, 0u8), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    hi - lo <= 2
}

/// How far the content moved up between `previous` and `next`: the `dy` for
/// which `next[y]` looks most like `previous[y + dy]` over the moving part of
/// the area, counting only rows that are not a single colour. `Some(0)` when
/// it did not move; `None` when no offset is convincing.
fn offset(
    previous: &[Profile],
    next: &[Profile],
    top: usize,
    bottom: usize,
    hint: usize,
) -> Option<usize> {
    let end = previous.len().saturating_sub(bottom);
    let moving = end.saturating_sub(top);
    if moving < 16 {
        return None;
    }
    let informative: Vec<usize> = (top..end).filter(|&y| !is_plain(&next[y])).collect();
    // Enough overlap to be sure: an eighth of the moving part, at least 8 rows.
    let min_overlap = (moving / 8).max(8);
    let max_dy = moving - min_overlap;
    // Mean difference per row for one offset, over every `step`-th
    // informative row that still overlaps.
    let cost = |dy: usize, step: usize| -> Option<f32> {
        let (mut seen, mut total) = (0u32, 0u64);
        for &y in informative.iter().step_by(step) {
            if y + dy >= end {
                break;
            }
            seen += 1;
            total += distance(&next[y], &previous[y + dy]) as u64;
        }
        (seen >= 6).then(|| total as f32 / seen as f32)
    };
    // A coarse look at every offset, then a full one at the best few.
    let mut coarse: Vec<(f32, usize)> = (0..=max_dy)
        .filter_map(|dy| cost(dy, 4).map(|c| (c, dy)))
        .collect();
    if coarse.is_empty() {
        return None;
    }
    coarse.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Every row for the best few: a sample of rows can agree by chance in
    // regular content (a cost of 0 on every fourth row, 400 on all of them).
    let mut fine: Vec<(f32, usize)> = coarse
        .iter()
        .take(16)
        .filter_map(|&(_, dy)| cost(dy, 1).map(|c| (c, dy)))
        .collect();
    fine.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.abs_diff(hint).cmp(&b.1.abs_diff(hint)))
    });
    let (best, dy) = *fine.first()?;
    if best > MATCH_PER_SLICE * SLICES as f32 {
        return None;
    }
    // The best rival not next to the winner, measured the same way.
    let rival = fine
        .iter()
        .find(|&&(_, d)| d.abs_diff(dy) > 2)
        .map(|&(c, _)| c);
    if let Some(r) = rival
        && best > CLEAR_WIN * r
        && best > STILL as f32
    {
        return None;
    }
    Some(dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tall invented page: rows of varied "text" and blank lines.
    fn page(width: u32, height: u32) -> Image {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let line = y / 18;
                let ink = (y % 18) < 12 && line % 5 != 4 && (x * 7 + line * 13) % 23 < 15;
                let v = if ink {
                    (40 + (line * 37 + x) % 90) as u8
                } else {
                    250
                };
                pixels.extend_from_slice(&[v, v.wrapping_add(3), v.wrapping_sub(2), 255]);
            }
        }
        Image {
            width,
            height,
            pixels,
        }
    }

    /// The view of `page` scrolled to `at`, `h` rows tall, with an optional
    /// header and footer of `band` rows that never move, and a scroll bar.
    fn view(page: &Image, at: u32, h: u32, band: u32) -> Image {
        let w = page.width;
        let mut img = page.crop(crate::util::Rect::new(0, at as i32, w as i32, h as i32));
        let stride = (w * 4) as usize;
        for y in 0..band {
            for yy in [y, h - 1 - y] {
                let row = &mut img.pixels[yy as usize * stride..(yy as usize + 1) * stride];
                for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let v = if (x / 9) % 2 == 0 { 30 } else { 200 };
                    px.copy_from_slice(&[v, 90, 160, 255]);
                }
            }
        }
        // A scroll bar thumb that follows the position, in the last 12 px.
        let thumb = (at * h / page.height) as usize;
        for y in 0..h as usize {
            for x in w as usize - 12..w as usize {
                let on = y >= thumb && y < thumb + 40;
                let v = if on { 120 } else { 235 };
                img.pixels[y * stride + x * 4..y * stride + x * 4 + 4]
                    .copy_from_slice(&[v, v, v, 255]);
            }
        }
        img
    }

    /// The view at a fractional position, as a browser at 125 % draws it:
    /// every row a blend of the two page rows it falls between.
    fn view_frac(page: &Image, at: f32, h: u32) -> Image {
        let stride = (page.width * 4) as usize;
        let mut pixels = Vec::with_capacity(stride * h as usize);
        for y in 0..h {
            let p = at + y as f32;
            let y0 = p.floor() as usize;
            let f = p - y0 as f32;
            let (a, b) = (
                &page.pixels[y0 * stride..(y0 + 1) * stride],
                &page.pixels[(y0 + 1) * stride..(y0 + 2) * stride],
            );
            pixels.extend(
                a.iter()
                    .zip(b)
                    .map(|(&u, &v)| (u as f32 * (1.0 - f) + v as f32 * f).round() as u8),
            );
        }
        Image {
            width: page.width,
            height: h,
            pixels,
        }
    }

    #[test]
    fn fractional_scrolling_still_stitches() {
        let full = page(300, 3000);
        let h = 400;
        let mut s = Stitcher::new(&view_frac(&full, 0.0, h), 20_000);
        let mut at = 0.0f32;
        for step in [37.4, 120.6, 99.8, 200.2, 64.5, 150.3, 125.0] {
            at += step;
            let r = s.push(&view_frac(&full, at, h));
            assert!(matches!(r, Step::Added(_)), "at {at}: {r:?}");
        }
        // Each step is rounded to a whole row: off by at most half a row each.
        let expected = (at + h as f32).round() as i64;
        assert!(
            (s.rows() as i64 - expected).abs() <= 4,
            "{} rows, expected about {expected}",
            s.rows()
        );
    }

    /// A view with a window's edges inside the area: a rounded corner at the
    /// bottom left and a scroll bar's arrow at the bottom right, both fixed
    /// on the screen while the page moves.
    fn view_with_chrome(page: &Image, at: u32, h: u32) -> Image {
        let mut img = view(page, at, h, 0);
        let w = img.width as usize;
        let stride = w * 4;
        // The window's border and the page's margin: plain.
        for y in 0..h as usize {
            for x in 0..4 {
                img.pixels[y * stride + x * 4..y * stride + x * 4 + 4]
                    .copy_from_slice(&[250, 253, 248, 255]);
            }
        }
        for y in h as usize - 8..h as usize {
            for x in 0..4 {
                img.pixels[y * stride + x * 4..y * stride + x * 4 + 4]
                    .copy_from_slice(&[20, 20, 20, 255]);
            }
            for x in w - 9..w - 4 {
                img.pixels[y * stride + x * 4..y * stride + x * 4 + 4]
                    .copy_from_slice(&[60, 60, 60, 255]);
            }
        }
        img
    }

    #[test]
    fn a_windows_own_edges_are_trimmed_not_repeated() {
        let full = page(300, 3000);
        let h = 400;
        let mut s = Stitcher::new(&view_with_chrome(&full, 0, h), 10_000);
        let mut at = 0;
        for step in [90, 120, 75, 160, 100] {
            at += step;
            assert!(matches!(
                s.push(&view_with_chrome(&full, at, h)),
                Step::Added(_)
            ));
        }
        let out = s.image();
        let (h_out, os, ps) = (out.height as usize, out.width as usize * 4, 300 * 4);
        // The scroll bar, whose thumb moves on its own, is trimmed.
        assert!(out.width <= 300 - 12, "width {}", out.width);
        // The corner and the arrow sit in the last rows only, never repeated.
        for y in 0..h_out {
            let row = &out.pixels[y * os..(y + 1) * os];
            let marked = row
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| *p == [20, 20, 20, 255] || *p == [60, 60, 60, 255]);
            assert!(
                !marked || y >= h_out - 16,
                "row {y} of {h_out} repeats the window's edge"
            );
        }
        // What remains is the page, row for row (with or without the
        // plain margin on the left).
        for y in [0usize, 500, 900, h_out - 40] {
            let page = &full.pixels[y * ps + 4 * 4..y * ps + 204 * 4];
            let found = [0usize, 4]
                .iter()
                .any(|&cut| &out.pixels[y * os + (4 - cut) * 4..y * os + (204 - cut) * 4] == page);
            assert!(found, "row {y}");
        }
    }

    #[test]
    fn scrolling_down_rebuilds_the_page() {
        let full = page(300, 2000);
        let h = 400;
        let mut s = Stitcher::new(&view(&full, 0, h, 0), 10_000);
        let mut at = 0;
        for step in [37, 120, 5, 200, 64, 300, 150] {
            at += step;
            assert_eq!(
                s.push(&view(&full, at, h, 0)),
                Step::Added(step as usize),
                "at {at}"
            );
        }
        let out = s.image();
        assert_eq!(out.height, at + h);
        let expected = full.crop(crate::util::Rect::new(0, 0, 300, (at + h) as i32));
        // Everything but the scroll bar column is the page itself.
        let (os, ps) = (out.width as usize * 4, 300 * 4);
        for y in 0..out.height as usize {
            assert_eq!(
                &out.pixels[y * os..y * os + 280 * 4],
                &expected.pixels[y * ps..y * ps + 280 * 4],
                "row {y}"
            );
        }
    }

    #[test]
    fn a_fixed_header_and_footer_appear_once() {
        let full = page(300, 2000);
        let (h, band) = (400, 50);
        let mut s = Stitcher::new(&view(&full, 0, h, band), 10_000);
        for at in [90, 180, 270] {
            assert_eq!(s.push(&view(&full, at, h, band)), Step::Added(90));
        }
        let out = s.image();
        assert_eq!(out.height, 270 + h);
        // The footer closes the image; the rows above it are the page. (The
        // scroll bar the view draws is trimmed, so rows are narrower.)
        let (os, ps) = (out.width as usize * 4, 300 * 4);
        let last_page_row = (out.height - band - 1) as usize;
        let page_row = 270 + (h - band - 1) as usize;
        assert_eq!(
            &out.pixels[last_page_row * os..last_page_row * os + 100 * 4],
            &full.pixels[page_row * ps..page_row * ps + 100 * 4]
        );
    }

    #[test]
    fn no_movement_adds_nothing_and_a_jump_is_lost() {
        let full = page(300, 3000);
        let h = 400;
        let mut s = Stitcher::new(&view(&full, 0, h, 0), 10_000);
        assert_eq!(s.push(&view(&full, 0, h, 0)), Step::Same);
        // Further than a whole view: nothing in common.
        assert_eq!(s.push(&view(&full, 900, h, 0)), Step::Lost);
        // Scrolling back within reach of the last good frame continues.
        assert_eq!(s.push(&view(&full, 100, h, 0)), Step::Added(100));
        assert_eq!(s.rows(), 500);
    }

    #[test]
    fn the_height_limit_stops_the_image() {
        let full = page(200, 3000);
        let h = 300;
        let mut s = Stitcher::new(&view(&full, 0, h, 0), 500);
        assert_eq!(s.push(&view(&full, 150, h, 0)), Step::Added(150));
        assert_eq!(s.push(&view(&full, 300, h, 0)), Step::Full);
        assert_eq!(s.rows(), 500);
        assert!(s.is_full());
        assert_eq!(s.push(&view(&full, 400, h, 0)), Step::Full);
    }

    #[test]
    fn a_strip_a_few_rows_tall_is_refused_not_a_panic() {
        let full = page(300, 2000);
        for h in [1, 3, 4, 20] {
            let mut s = Stitcher::new(&view(&full, 0, h, 0), 1_000);
            assert_eq!(s.push(&view(&full, 30, h, 0)), Step::Lost, "{h} rows");
        }
    }

    #[test]
    fn a_blank_area_never_invents_movement() {
        let blank = Image {
            width: 200,
            height: 300,
            pixels: vec![255; 200 * 300 * 4],
        };
        let mut s = Stitcher::new(&blank, 5_000);
        assert_eq!(s.push(&blank), Step::Same);
    }
}
