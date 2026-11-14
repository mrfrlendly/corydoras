//! Inventory photos are normalized JPEGs. Stock items can share one photo and its thumbnail.
use crate::{analysis, Library};
use anyhow::{anyhow, ensure, Context, Result};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use ring::digest::{digest, SHA256};
use rusqlite::{params, OptionalExtension, Transaction};
use std::io::Cursor;

pub const MAX_PHOTO_BYTES: usize = 12 * 1024 * 1024;
const MAX_PIXELS: u64 = 50_000_000;
const MAX_DECODED_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Clone, Copy)]
pub enum InventoryPhotoChange<'a> {
    Keep,
    Remove,
    Replace(&'a [u8]),
}

pub(crate) struct PreparedPhoto {
    pub key: String,
    pub image: Vec<u8>,
    pub thumbnail: Vec<u8>,
}
pub(crate) fn decode_photo(bytes: &[u8]) -> Result<DynamicImage> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_PHOTO_BYTES,
        "Choose a photo up to 12 MiB."
    );
    let format = image::guess_format(bytes).context("Choose a JPEG, PNG, or WebP photo.")?;
    ensure!(
        matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
        ),
        "Choose a JPEG, PNG, or WebP photo."
    );
    let mut limits = Limits::default();
    limits.max_image_width = Some(10_000);
    limits.max_image_height = Some(10_000);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .context("Cannot decode this photo. Choose a smaller JPEG, PNG, or WebP photo.")?;
    let (w, h) = decoder.dimensions();
    ensure!(
        u64::from(w) * u64::from(h) <= MAX_PIXELS && decoder.total_bytes() <= MAX_DECODED_BYTES,
        "The photo is too large to decode. Choose a photo up to 50 megapixels."
    );
    let orientation = decoder.orientation().ok();
    let mut image =
        DynamicImage::from_decoder(decoder).context("Cannot read the pixels in this photo.")?;
    if let Some(o) = orientation {
        image.apply_orientation(o);
    }
    Ok(image)
}

pub(crate) fn prepare_photo(bytes: &[u8]) -> Result<PreparedPhoto> {
    let image = decode_photo(bytes)?;
    let preview = image.thumbnail(192, 192);
    let thumbnail = analysis::encode_jpeg(&preview, 192, 82)?;
    let image = analysis::encode_jpeg(&image, 2048, 90)?;
    let key = digest(&SHA256, &image)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(PreparedPhoto {
        key,
        image,
        thumbnail,
    })
}

pub(crate) fn store_photo(tx: &Transaction<'_>, photo: &PreparedPhoto) -> Result<()> {
    tx.execute(
        "INSERT INTO workshop_photos (key,image,thumbnail) VALUES (?1,?2,?3)
                ON CONFLICT(key) DO NOTHING",
        params![photo.key, photo.image, photo.thumbnail],
    )?;
    Ok(())
}

impl Library {
    pub fn inventory_photo(&self, key: &str, thumbnail: bool) -> Result<Vec<u8>> {
        ensure!(
            key.len() == 64
                && key
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid inventory photo identifier."
        );
        let sql = if thumbnail {
            "SELECT thumbnail FROM workshop_photos WHERE key=?1"
        } else {
            "SELECT image FROM workshop_photos WHERE key=?1"
        };
        self.c()
            .query_row(sql, [key], |row| row.get(0))
            .optional()?
            .ok_or_else(|| anyhow!("This inventory photo no longer exists."))
    }
}
