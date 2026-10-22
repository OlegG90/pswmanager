//! A TOTP secret read from a QR code in a picture (#279): the code a site
//! shows when 2FA is set up, snipped or saved as an image. What it holds is
//! an `otpauth://totp/` URI; the editor takes it as if it had been typed.

use crate::otp;
use image::{GrayImage, ImageReader, Limits};
use std::io::Cursor;
use zeroize::Zeroizing;

/// Google Authenticator's export, many accounts in one code (#281).
const MIGRATION: &str = "otpauth-migration:";
/// The largest picture file read, and the most pixels a side: a screenshot of
/// several screens fits, a picture made to fill the memory does not.
const MAX_FILE: u64 = 20 << 20;
const MAX_SIDE: u32 = 16_384;

/// Refuses a picture file too big to read for a QR code, before it is read.
pub fn check_size(len: u64) -> Result<(), String> {
    if len > MAX_FILE {
        return Err(format!("Pictures over {} MB are not read for a QR code", MAX_FILE >> 20));
    }
    Ok(())
}

/// The TOTP URI in a PNG or JPEG image. The file's bytes are the caller's to
/// wipe.
pub fn from_image(bytes: &[u8]) -> Result<Zeroizing<String>, String> {
    check_size(bytes.len() as u64)?;
    let unreadable = |_| "Not a picture PswManager can read (PNG or JPEG)".to_string();
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(unreadable)?;
    let mut limits = Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    reader.limits(limits);
    from_gray(reader.decode().map_err(|_| "Not a picture PswManager can read (PNG or JPEG), or too big".to_string())?.to_luma8())
}

/// The TOTP URI in an image as the clipboard holds it: `width` × `height`
/// pixels of red, green, blue and alpha. A transparent pixel counts as white,
/// the page it would be shown on.
pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Zeroizing<String>, String> {
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err("The picture is too big to read for a QR code".into());
    }
    let gray: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, a]| {
            let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
            ((luma * u32::from(a) + 255 * (255 - u32::from(a))) / 255) as u8
        })
        .collect();
    from_gray(GrayImage::from_raw(width, height, gray).ok_or("The picture's size does not match its pixels")?)
}

/// The first code in the picture that holds a TOTP URI; failing that, why the
/// codes found do not (the most telling reason).
fn from_gray(picture: GrayImage) -> Result<Zeroizing<String>, String> {
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(picture.width() as usize, picture.height() as usize, |x, y| {
        picture.get_pixel(x as u32, y as u32).0[0]
    });
    let mut why = "No QR code found in the picture".to_string();
    for grid in prepared.detect_grids() {
        let Ok((_, content)) = grid.decode() else { continue };
        let content = Zeroizing::new(content);
        if content.get(..MIGRATION.len()).is_some_and(|start| start.eq_ignore_ascii_case(MIGRATION)) {
            why = "This is Google Authenticator's export: importing it is not supported yet".into();
        } else if otp::is_uri(&content) {
            match otp::Totp::parse(&content) {
                Ok(_) => return Ok(content),
                Err(e) => why = e,
            }
        } else {
            why = "The QR code holds no TOTP secret (otpauth://totp/)".into();
        }
    }
    Err(why)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    /// `text` as a QR code: black modules four pixels wide on white, with
    /// the quiet zone around them.
    fn picture(text: &str) -> GrayImage {
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let (modules, scale, quiet) = (code.width() as u32, 4, 4);
        let side = (modules + 2 * quiet) * scale;
        let colors = code.to_colors();
        GrayImage::from_fn(side, side, |x, y| {
            let (mx, my) = ((x / scale).wrapping_sub(quiet), (y / scale).wrapping_sub(quiet));
            let dark = mx < modules && my < modules && colors[(my * modules + mx) as usize] == qrcode::Color::Dark;
            image::Luma([if dark { 0 } else { 255 }])
        })
    }

    fn png(picture: &GrayImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(picture.as_raw(), picture.width(), picture.height(), image::ExtendedColorType::L8)
            .unwrap();
        bytes
    }

    const URI: &str = "otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP&issuer=Example";

    #[test]
    fn reads_the_uri_from_a_png_and_from_clipboard_pixels() {
        let code = picture(URI);
        assert_eq!(from_image(&png(&code)).unwrap().as_str(), URI);
        let rgba: Vec<u8> = code.pixels().flat_map(|p| [p.0[0], p.0[0], p.0[0], 255]).collect();
        assert_eq!(from_rgba(code.width(), code.height(), &rgba).unwrap().as_str(), URI);
        // Black on a transparent background, as some snips are.
        let clear: Vec<u8> = code.pixels().flat_map(|p| if p.0[0] == 0 { [0, 0, 0, 255] } else { [0, 0, 0, 0] }).collect();
        assert_eq!(from_rgba(code.width(), code.height(), &clear).unwrap().as_str(), URI);
    }

    #[test]
    fn a_totp_code_is_found_beside_another() {
        let (export, totp) = (picture("otpauth-migration://offline?data=AAAA"), picture(URI));
        let mut both = GrayImage::from_pixel(export.width() + totp.width(), export.height().max(totp.height()), image::Luma([255]));
        image::imageops::replace(&mut both, &export, 0, 0);
        image::imageops::replace(&mut both, &totp, i64::from(export.width()), 0);
        assert_eq!(from_image(&png(&both)).unwrap().as_str(), URI);
    }

    #[test]
    fn says_what_the_picture_lacks() {
        let blank = GrayImage::from_pixel(64, 64, image::Luma([255]));
        assert_eq!(from_image(&png(&blank)).unwrap_err(), "No QR code found in the picture");
        assert_eq!(from_image(&png(&picture("https://example.com"))).unwrap_err(), "The QR code holds no TOTP secret (otpauth://totp/)");
        assert!(from_image(&png(&picture("otpauth-migration://offline?data=AAAA"))).unwrap_err().contains("Google Authenticator"));
        assert_eq!(from_image(&png(&picture("otpauth://hotp/x?secret=JBSWY3DP"))).unwrap_err(), "Only time-based codes (otpauth://totp/) are supported");
        assert!(from_image(b"not a picture").is_err());
        assert!(from_rgba(MAX_SIDE + 1, 1, &[]).is_err());
        assert!(from_rgba(10, 10, &[0; 12]).is_err());
    }
}
