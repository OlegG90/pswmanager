//! A TOTP secret read from a QR code in a picture (#279): the code a site
//! shows when 2FA is set up, snipped or saved as an image. What it holds is
//! an `otpauth://totp/` URI; the editor takes it as if it had been typed.

use crate::otp;
use image::GrayImage;
use zeroize::Zeroizing;

/// Google Authenticator's export, many accounts in one code (#281).
const MIGRATION: &str = "otpauth-migration:";

/// The TOTP URI in a PNG or JPEG image. The file's bytes are the caller's to
/// wipe.
pub fn from_image(bytes: &[u8]) -> Result<Zeroizing<String>, String> {
    let picture = image::load_from_memory(bytes).map_err(|_| "Not a picture PswManager can read (PNG or JPEG)".to_string())?;
    from_gray(picture.to_luma8())
}

/// The TOTP URI in an image as the clipboard holds it: `width` × `height`
/// pixels of red, green, blue and alpha.
pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Zeroizing<String>, String> {
    let gray: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|[r, g, b, _]| ((u32::from(*r) * 299 + u32::from(*g) * 587 + u32::from(*b) * 114) / 1000) as u8).collect();
    from_gray(GrayImage::from_raw(width, height, gray).ok_or("The picture's size does not match its pixels")?)
}

fn from_gray(picture: GrayImage) -> Result<Zeroizing<String>, String> {
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(picture.width() as usize, picture.height() as usize, |x, y| {
        picture.get_pixel(x as u32, y as u32).0[0]
    });
    let mut found_other = false;
    for grid in prepared.detect_grids() {
        let Ok((_, content)) = grid.decode() else { continue };
        let content = Zeroizing::new(content);
        if content.get(..MIGRATION.len()).is_some_and(|start| start.eq_ignore_ascii_case(MIGRATION)) {
            return Err("This is Google Authenticator's export: importing it is not supported yet".into());
        }
        if otp::is_uri(&content) && otp::Totp::parse(&content).is_ok() {
            return Ok(content);
        }
        found_other = true;
    }
    Err(if found_other { "The QR code holds no TOTP secret (otpauth://totp/)" } else { "No QR code found in the picture" }.into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::ImageEncoder;

    /// `text` as a QR code: black modules four pixels wide on white, with
    /// the quiet zone around them.
    pub(crate) fn picture(text: &str) -> GrayImage {
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
        let picture = picture(URI);
        assert_eq!(from_image(&png(&picture)).unwrap().as_str(), URI);
        let rgba: Vec<u8> = picture.pixels().flat_map(|p| [p.0[0], p.0[0], p.0[0], 255]).collect();
        assert_eq!(from_rgba(picture.width(), picture.height(), &rgba).unwrap().as_str(), URI);
    }

    #[test]
    fn says_what_the_picture_lacks() {
        let blank = GrayImage::from_pixel(64, 64, image::Luma([255]));
        assert_eq!(from_image(&png(&blank)).unwrap_err(), "No QR code found in the picture");
        assert_eq!(from_image(&png(&picture("https://example.com"))).unwrap_err(), "The QR code holds no TOTP secret (otpauth://totp/)");
        assert!(from_image(&png(&picture("otpauth-migration://offline?data=AAAA"))).unwrap_err().contains("Google Authenticator"));
        assert!(from_image(b"not a picture").is_err());
        assert!(from_rgba(10, 10, &[0; 12]).is_err());
    }
}
