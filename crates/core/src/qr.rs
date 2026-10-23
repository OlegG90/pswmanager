//! A TOTP secret read from a QR code in a picture (#279): the code a site
//! shows when 2FA is set up, snipped or saved as an image. What it holds is
//! an `otpauth://totp/` URI; the editor takes it as if it had been typed.
//! Google Authenticator's export codes are read the same way for its import
//! (#281, [crate::gauth]).

use crate::{gauth, otp};
use image::{ImageReader, Limits};
use std::io::Cursor;
use zeroize::Zeroizing;

/// What a code read is checked with: the text it holds, or why not.
type Wanted = fn(Zeroizing<String>) -> Result<Zeroizing<String>, String>;
/// A reader of the code wanted in pixels ([from_rgba], [export_from_rgba])
/// or in a picture file ([from_image], [export_from_image]).
pub type FromPixels = fn(u32, u32, &[u8]) -> Result<Zeroizing<String>, String>;
pub type FromFile = fn(&[u8]) -> Result<Zeroizing<String>, String>;

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
    image_codes(bytes, from_text)
}

/// The TOTP URI in an image as the clipboard holds it: `width` × `height`
/// pixels of red, green, blue and alpha.
pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Zeroizing<String>, String> {
    rgba_codes(width, height, rgba, from_text)
}

/// The text of a Google Authenticator export code in a PNG or JPEG image.
pub fn export_from_image(bytes: &[u8]) -> Result<Zeroizing<String>, String> {
    image_codes(bytes, export_text)
}

/// The text of a Google Authenticator export code in the clipboard's pixels.
pub fn export_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Zeroizing<String>, String> {
    rgba_codes(width, height, rgba, export_text)
}

fn image_codes(bytes: &[u8], wanted: Wanted) -> Result<Zeroizing<String>, String> {
    check_size(bytes.len() as u64)?;
    const UNREADABLE: &str = "Not a picture PswManager can read (PNG or JPEG), or too big";
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| UNREADABLE.to_string())?;
    let mut limits = Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    reader.limits(limits);
    let gray = reader.decode().map_err(|_| UNREADABLE.to_string())?.to_luma8();
    let (width, raw) = (gray.width() as usize, gray.as_raw());
    read_codes(gray.width(), gray.height(), |x, y| raw[y * width + x], wanted)
}

/// A transparent pixel counts as white, the page it would be shown on.
fn rgba_codes(width: u32, height: u32, rgba: &[u8], wanted: Wanted) -> Result<Zeroizing<String>, String> {
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err("The picture is too big to read for a QR code".into());
    }
    let pixels = rgba.as_chunks::<4>().0;
    if pixels.len() != width as usize * height as usize {
        return Err("The picture's size does not match its pixels".into());
    }
    read_codes(width, height, |x, y| {
        let [r, g, b, a] = pixels[y * width as usize + x].map(u32::from);
        let luma = (r * 299 + g * 587 + b * 114) / 1000;
        ((luma * a + 255 * (255 - a)) / 255) as u8
    }, wanted)
}

/// The first code in the picture `wanted` takes; failing that, why the codes
/// found are not (the last reason). `gray` gives each pixel's brightness,
/// read straight into the decoder.
fn read_codes(width: u32, height: u32, gray: impl FnMut(usize, usize) -> u8, wanted: Wanted) -> Result<Zeroizing<String>, String> {
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(width as usize, height as usize, gray);
    let mut why = "No QR code found in the picture".to_string();
    for grid in prepared.detect_grids() {
        let Ok((_, content)) = grid.decode() else { continue };
        match wanted(Zeroizing::new(content)) {
            Ok(uri) => return Ok(uri),
            Err(e) => why = e,
        }
    }
    Err(why)
}

/// What a QR code holds, when it is a TOTP URI; otherwise why not. The
/// camera's codes come here as text (#280).
pub fn from_text(content: Zeroizing<String>) -> Result<Zeroizing<String>, String> {
    if gauth::is_export(&content) {
        return Err("This is Google Authenticator's export: import it with Tools → Import from Google Authenticator".into());
    }
    if !otp::is_uri(&content) {
        return Err("The QR code holds no TOTP secret (otpauth://totp/)".into());
    }
    otp::Totp::parse(&content)?;
    Ok(content)
}

/// What a code holds, when it is one of Google Authenticator's export.
pub fn export_text(content: Zeroizing<String>) -> Result<Zeroizing<String>, String> {
    if gauth::is_export(&content) {
        return Ok(content);
    }
    Err(if otp::is_uri(&content) {
        "This code is one account's: read it in the entry's TOTP field".into()
    } else {
        "This is not a code of Google Authenticator's export".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, ImageEncoder};

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
        let export = "otpauth-migration://offline?data=AAAA";
        assert_eq!(export_from_image(&png(&picture(export))).unwrap().as_str(), export);
        assert!(export_from_image(&png(&picture(URI))).unwrap_err().contains("one account's"));
        assert_eq!(from_text(Zeroizing::new(URI.into())).unwrap().as_str(), URI);
        assert!(from_text(Zeroizing::new("otpauth://totp/x".into())).unwrap_err().contains("no secret"));
        assert!(from_rgba(MAX_SIDE + 1, 1, &[]).is_err());
        assert!(from_rgba(10, 10, &[0; 12]).is_err());
    }
}
