/// Largest JPEG embedded in a camera RAW (or any other container).
///
/// The scan is a byte search for SOI (`FF D8`) and EOI (`FF D9`). The longest
/// marker pair above 2 KiB is the preview manufacturers store for the back of
/// the camera. Embedding that preview avoids demosaicing the sensor data.
pub fn largest_embedded_jpeg(bytes: &[u8]) -> Option<&[u8]> {
    let mut best: Option<&[u8]> = None;
    let mut index = 0;
    while index + 3 < bytes.len() {
        if bytes[index] == 0xFF && bytes[index + 1] == 0xD8 {
            if let Some(end) = find_eoi(&bytes[index + 2..]) {
                let jpeg = &bytes[index..index + 2 + end + 2];
                let longer = best.map(|found| jpeg.len() > found.len()).unwrap_or(true);
                if longer && jpeg.len() > 256 {
                    best = Some(jpeg);
                }
                index += jpeg.len().max(2);
                continue;
            }
        }
        index += 1;
    }
    best
}

fn find_eoi(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|pair| pair == [0xFF, 0xD9])
}
