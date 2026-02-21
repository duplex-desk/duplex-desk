use makepad_components::makepad_widgets::{Cx, Texture, TextureFormat, TextureUpdated};

#[derive(Default)]
pub struct VideoFrameTexture {
    texture: Option<Texture>,
    frame_width: usize,
    frame_height: usize,
}

impl VideoFrameTexture {
    pub fn texture(&self) -> Option<Texture> {
        self.texture.clone()
    }

    pub fn update_frame(
        &mut self,
        cx: &mut Cx,
        bgra: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> bool {
        if width == 0 || height == 0 {
            return false;
        }

        let expected_row_bytes = width.saturating_mul(4);
        if stride < expected_row_bytes {
            return false;
        }

        let required_bytes = stride.saturating_mul(height);
        if bgra.len() < required_bytes {
            return false;
        }

        if self.texture.is_none() || self.frame_width != width || self.frame_height != height {
            self.texture = Some(Texture::new_with_format(
                cx,
                TextureFormat::VecBGRAu8_32 {
                    width,
                    height,
                    data: Some(vec![0; width.saturating_mul(height)]),
                    updated: TextureUpdated::Full,
                },
            ));
            self.frame_width = width;
            self.frame_height = height;
        }

        let mut packed = Vec::with_capacity(width.saturating_mul(height));
        if stride == expected_row_bytes {
            for px in bgra.chunks_exact(4).take(width.saturating_mul(height)) {
                packed.push(u32::from_le_bytes([px[0], px[1], px[2], px[3]]));
            }
        } else {
            for row in 0..height {
                let start = row.saturating_mul(stride);
                let end = start.saturating_add(expected_row_bytes);
                for px in bgra[start..end].chunks_exact(4) {
                    packed.push(u32::from_le_bytes([px[0], px[1], px[2], px[3]]));
                }
            }
        }

        if let Some(texture) = &self.texture {
            texture.swap_vec_u32(cx, &mut packed);
            true
        } else {
            false
        }
    }
}
