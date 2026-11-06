use super::*;

pub(super) struct Rgba {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}
impl Rgba {
    pub fn blank(width: u32, height: u32) -> Result<Self, String> {
        let size = pixel_bytes(width, height)?;
        Ok(Self {
            width,
            height,
            bytes: vec![0; size],
        })
    }
    pub fn paste(&mut self, input: &Rgba, x: u32, y: u32, cut: u32) -> Result<(), String> {
        if cut >= input.height
            || x.checked_add(input.width).is_none_or(|w| w > self.width)
            || y.checked_add(input.height - cut)
                .is_none_or(|h| h > self.height)
        {
            return Err("A frame exceeds the bounded image canvas.".into());
        }
        for row in cut..input.height {
            let src = (row * input.width * 4) as usize;
            let dst = (((row - cut + y) * self.width + x) * 4) as usize;
            let count = (input.width * 4) as usize;
            self.bytes[dst..dst + count].copy_from_slice(&input.bytes[src..src + count]);
        }
        Ok(())
    }
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self, String> {
        if x.checked_add(width).is_none_or(|w| w > self.width)
            || y.checked_add(height).is_none_or(|h| h > self.height)
        {
            return Err("The frame crop exceeds the source image.".into());
        }
        let mut out = Self::blank(width, height)?;
        for row in 0..height {
            let src = (((row + y) * self.width + x) * 4) as usize;
            let dst = (row * width * 4) as usize;
            let count = (width * 4) as usize;
            out.bytes[dst..dst + count].copy_from_slice(&self.bytes[src..src + count]);
        }
        Ok(out)
    }
    pub fn flatten(&mut self, color: [u8; 3]) {
        for pixel in self.bytes.chunks_exact_mut(4) {
            let alpha = pixel[3] as u32;
            for c in 0..3 {
                pixel[c] =
                    ((pixel[c] as u32 * alpha + color[c] as u32 * (255 - alpha) + 127) / 255) as u8;
            }
            pixel[3] = 255;
        }
    }
    pub fn thumbnail(&self, size: u32) -> Result<Self, String> {
        let mut out = Self::blank(size, size)?;
        let scale = (size as f64 / self.width as f64).min(size as f64 / self.height as f64);
        let w = (self.width as f64 * scale).round().max(1.0) as u32;
        let h = (self.height as f64 * scale).round().max(1.0) as u32;
        let x = (size - w) / 2;
        let y = (size - h) / 2;
        for row in 0..h {
            for col in 0..w {
                let src =
                    (((row * self.height / h) * self.width + col * self.width / w) * 4) as usize;
                let dst = (((row + y) * size + col + x) * 4) as usize;
                out.bytes[dst..dst + 4].copy_from_slice(&self.bytes[src..src + 4]);
            }
        }
        Ok(out)
    }
}

pub(super) fn suggest_overlap(
    first: &Rgba,
    second: &Rgba,
    index: usize,
    cancel: &watch::Receiver<bool>,
) -> Result<Suggestion, String> {
    if first.width != second.width {
        return Err("Overlap samples must have the same width.".into());
    }
    let maximum = first.height.min(second.height).saturating_sub(16).min(4096);
    if maximum < 8 {
        return Ok(Suggestion {
            index,
            cut: 0,
            score: 0.0,
            ambiguous: true,
        });
    }
    let mut candidates = Vec::new();
    for cut in 8..=maximum {
        if cut % 128 == 0 && *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let mut error = 0u64;
        let mut samples = 0u64;
        for sample in 0..16 {
            let row = sample * (cut - 1) / 15;
            for column in 0..16 {
                let x = column * (first.width - 1) / 15;
                let a = (((first.height - cut + row) * first.width + x) * 4) as usize;
                let b = ((row * second.width + x) * 4) as usize;
                for c in 0..3 {
                    let av = (first.bytes[a + c] as u32 * first.bytes[a + 3] as u32
                        + 255 * (255 - first.bytes[a + 3] as u32)
                        + 127)
                        / 255;
                    let bv = (second.bytes[b + c] as u32 * second.bytes[b + 3] as u32
                        + 255 * (255 - second.bytes[b + 3] as u32)
                        + 127)
                        / 255;
                    error += av.abs_diff(bv) as u64;
                    samples += 1;
                }
            }
        }
        candidates.push((error as f64 / samples as f64, cut));
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (error, cut) = candidates[0];
    let competing = candidates
        .iter()
        .find(|(_, candidate)| candidate.abs_diff(cut) > 4)
        .map(|v| v.0)
        .unwrap_or(255.0);
    let mut total = 0f64;
    let mut squared = 0f64;
    let mut count = 0f64;
    for row in 0..cut {
        let offset = ((first.height - cut + row) * first.width * 4) as usize;
        let value = first.bytes[offset] as f64;
        total += value;
        squared += value * value;
        count += 1.0;
    }
    let variance = (squared / count - (total / count).powi(2)).max(0.0);
    let score = (1.0 - error / 255.0).clamp(0.0, 1.0);
    Ok(Suggestion {
        index,
        cut,
        score,
        ambiguous: score < 0.98 || competing - error < 2.0 || variance < 16.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(start: u32, height: u32) -> Rgba {
        let mut image = Rgba::blank(64, height).unwrap();
        for y in 0..height {
            for x in 0..64 {
                let seed = (start + y).wrapping_mul(1103515245).wrapping_add(x * 12345);
                let offset = ((y * 64 + x) * 4) as usize;
                image.bytes[offset..offset + 4].copy_from_slice(&[
                    (seed >> 17) as u8,
                    (seed >> 9) as u8,
                    (seed >> 1) as u8,
                    255,
                ]);
            }
        }
        image
    }
    #[test]
    fn overlap_detects_real_shared_rows_and_flags_flat_repeated_content() {
        let (_, cancel) = watch::channel(false);
        let first = rows(0, 100);
        let second = rows(80, 100);
        let guess = suggest_overlap(&first, &second, 1, &cancel).unwrap();
        assert_eq!(guess.cut, 20);
        assert_eq!(guess.score, 1.0);
        assert!(!guess.ambiguous);
        let flat = Rgba {
            width: 64,
            height: 100,
            bytes: vec![255; 64 * 100 * 4],
        };
        assert!(suggest_overlap(&flat, &flat, 1, &cancel).unwrap().ambiguous);
    }
    #[test]
    fn paste_crop_and_solid_background_preserve_alpha_semantics() {
        let mut image = Rgba::blank(2, 2).unwrap();
        image.bytes[..4].copy_from_slice(&[255, 0, 0, 128]);
        let mut canvas = Rgba::blank(4, 4).unwrap();
        canvas.paste(&image, 1, 1, 0).unwrap();
        assert_eq!(&canvas.bytes[20..24], &[255, 0, 0, 128]);
        let crop = canvas.crop(1, 1, 2, 2).unwrap();
        assert_eq!(crop.bytes, image.bytes);
        image.flatten([0, 0, 255]);
        assert_eq!(&image.bytes[..4], &[128, 0, 127, 255]);
        assert!(canvas.crop(3, 3, 2, 2).is_err());
    }
}
