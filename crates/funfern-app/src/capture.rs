use bevy::prelude::Image;
use bevy_egui::egui::Rect;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PixelCrop {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

pub fn encode_viewport_png(
    image: Image,
    logical_canvas: Rect,
    logical_viewport: Rect,
) -> Result<Vec<u8>, String> {
    let image = image
        .try_into_dynamic()
        .map_err(|error| format!("Could not read captured pixels: {error}"))?;
    encode_dynamic_viewport_png(image, logical_canvas, logical_viewport)
}

fn encode_dynamic_viewport_png(
    image: DynamicImage,
    logical_canvas: Rect,
    logical_viewport: Rect,
) -> Result<Vec<u8>, String> {
    let crop = pixel_crop(
        logical_canvas,
        logical_viewport,
        image.width(),
        image.height(),
    )?;
    let cropped = image
        .crop_imm(crop.x, crop.y, crop.width, crop.height)
        .to_rgba8();
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(cropped)
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|error| format!("Could not encode snapshot: {error}"))?;
    Ok(bytes.into_inner())
}

fn pixel_crop(
    logical_canvas: Rect,
    logical_viewport: Rect,
    image_width: u32,
    image_height: u32,
) -> Result<PixelCrop, String> {
    if image_width == 0
        || image_height == 0
        || !logical_canvas.is_finite()
        || !logical_viewport.is_finite()
        || logical_canvas.width() <= 0.0
        || logical_canvas.height() <= 0.0
    {
        return Err("Captured viewport has invalid dimensions".into());
    }
    let x_scale = image_width as f64 / logical_canvas.width() as f64;
    let y_scale = image_height as f64 / logical_canvas.height() as f64;
    let left = (((logical_viewport.left() - logical_canvas.left()) as f64 * x_scale).ceil() as i64)
        .clamp(0, image_width as i64) as u32;
    let right = (((logical_viewport.right() - logical_canvas.left()) as f64 * x_scale).floor()
        as i64)
        .clamp(0, image_width as i64) as u32;
    let top = (((logical_viewport.top() - logical_canvas.top()) as f64 * y_scale).ceil() as i64)
        .clamp(0, image_height as i64) as u32;
    let bottom = (((logical_viewport.bottom() - logical_canvas.top()) as f64 * y_scale).floor()
        as i64)
        .clamp(0, image_height as i64) as u32;
    if right <= left || bottom <= top {
        return Err("Captured viewport is empty".into());
    }
    Ok(PixelCrop {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgba, RgbaImage};

    #[test]
    fn logical_viewport_maps_to_physical_pixels() {
        let canvas = Rect::from_min_max((0.0, 0.0).into(), (100.0, 50.0).into());
        let viewport = Rect::from_min_max((10.0, 5.0).into(), (90.0, 45.0).into());
        assert_eq!(
            pixel_crop(canvas, viewport, 200, 100).unwrap(),
            PixelCrop {
                x: 20,
                y: 10,
                width: 160,
                height: 80,
            }
        );
    }

    #[test]
    fn fractional_scale_uses_only_pixels_inside_the_viewport() {
        let canvas = Rect::from_min_max((0.0, 0.0).into(), (100.0, 50.0).into());
        let viewport = Rect::from_min_max((10.2, 4.8).into(), (90.4, 45.3).into());
        assert_eq!(
            pixel_crop(canvas, viewport, 150, 75).unwrap(),
            PixelCrop {
                x: 16,
                y: 8,
                width: 119,
                height: 59,
            }
        );
    }

    #[test]
    fn viewport_crop_is_clamped_and_encoded_without_flipping() {
        let canvas = Rect::from_min_max((0.0, 0.0).into(), (4.0, 4.0).into());
        let viewport = Rect::from_min_max((-1.0, 1.0).into(), (3.0, 5.0).into());
        let source = RgbaImage::from_fn(4, 4, |x, y| Rgba([x as u8, y as u8, 17, 255]));
        let bytes = encode_dynamic_viewport_png(DynamicImage::ImageRgba8(source), canvas, viewport)
            .unwrap();
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png).unwrap();
        assert_eq!(decoded.dimensions(), (3, 3));
        assert_eq!(decoded.get_pixel(0, 0), Rgba([0, 1, 17, 255]));
        assert_eq!(decoded.get_pixel(2, 2), Rgba([2, 3, 17, 255]));
    }

    #[test]
    fn invalid_or_empty_crops_are_rejected() {
        let canvas = Rect::from_min_max((0.0, 0.0).into(), (100.0, 50.0).into());
        let outside = Rect::from_min_max((120.0, 10.0).into(), (140.0, 20.0).into());
        assert!(pixel_crop(canvas, outside, 200, 100).is_err());
        assert!(pixel_crop(Rect::ZERO, canvas, 200, 100).is_err());
        assert!(pixel_crop(canvas, canvas, 0, 100).is_err());
    }
}
