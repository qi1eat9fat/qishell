//! 壁纸预模糊：解码、缩放到上限、模糊，再转成 GPUI 期望的 BGRA 帧。
//!
//! 纯函数放在这里，便于单测；解码与模糊都在后台线程执行。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 最长边上限：大图先缩到这个尺寸，滑动模糊滑杆不会卡住 UI。
///
/// 模糊后看不到细节，1200px 在高 DPI 屏上也够铺满；比 1600 再省一半像素，
/// 解码 + 三遍盒式模糊在 debug 构建下约 300ms，release 更快。
pub const MAX_EDGE: u32 = 1200;

/// 一张壁纸的缓存键：路径、修改时间、大小与模糊半径任何一项变了都要重做。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallpaperKey {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: Option<u64>,
    pub blur: u8,
    pub max_edge: u32,
}

impl WallpaperKey {
    pub fn for_path(path: &Path, blur: u8) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            path: path.to_path_buf(),
            modified: metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            len: metadata.as_ref().map(|metadata| metadata.len()),
            blur: blur.min(30),
            max_edge: MAX_EDGE,
        }
    }
}

impl WallpaperKey {
    /// 同一个文件、同一个缩放上限，只是模糊半径不同。
    pub fn same_file(&self, other: &Self) -> bool {
        self.path == other.path
            && self.modified == other.modified
            && self.len == other.len
            && self.max_edge == other.max_edge
    }

    /// 底图缓存的文件名（不含扩展名）：只随图片文件与缩放上限变化。
    fn source_stem(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!(
            "{CACHE_VERSION}|{}|{:?}|{:?}|{}",
            self.path.display(),
            self.modified,
            self.len,
            self.max_edge
        ));
        let digest = hasher.finalize();
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// 磁盘缓存版本：像素格式或模糊算法变了就加一，旧文件自动失效。
const CACHE_VERSION: u32 = 1;

/// 数据目录里的缓存子目录名。
pub const CACHE_DIR: &str = "wallpaper-cache";

/// 一张壁纸的两份缓存文件：缩放后的底图、按当前模糊半径做好的成品。
///
/// 存在磁盘上是为了让第二次启动不必重新解码 4K 原图：读一份 3MB 的
/// 裸像素只要几毫秒，而解码加模糊要几百毫秒，期间用户会看到「先清晰、后
/// 变糊」的跳变。
pub struct CachePaths {
    pub source: PathBuf,
    pub blurred: PathBuf,
}

pub fn cache_paths(directory: &Path, key: &WallpaperKey) -> CachePaths {
    let stem = key.source_stem();
    CachePaths {
        source: directory.join(format!("{stem}.src")),
        // 缓存里存的是 RGBA，上屏前才换成 BGRA。
        blurred: directory.join(format!("{stem}-{}-{}.rgba", key.blur, key.max_edge)),
    }
}

/// 读一份缓存；文件缺失、损坏或尺寸对不上都返回 `None`，当作没命中。
pub fn read_cache(path: &Path) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let width = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if width == 0 || height == 0 {
        return None;
    }
    let expected = 8 + width as usize * height as usize * 4;
    if bytes.len() != expected {
        return None;
    }
    let pixels = bytes[8..].to_vec();
    image::RgbaImage::from_raw(width, height, pixels)
}

/// 整份写入：先写临时文件再改名，写一半退出也不会留下半张图。
pub fn write_cache(path: &Path, image: &image::RgbaImage) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let (width, height) = image.dimensions();
    let mut bytes = Vec::with_capacity(8 + width as usize * height as usize * 4);
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(image.as_raw());
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

/// 启动时同步读缓存：命中成品就直接上屏，底图有就一起带上。
///
/// 只给「内存里还什么都没有」的首次加载用；拖杆与换图都走后台线程，
/// 否则一次几 MB 的读盘会卡住输入。
pub fn read_ready_now(key: &WallpaperKey) -> Option<(Option<image::RgbaImage>, image::RgbaImage)> {
    let directory = crate::app::paths::data_dir().join(CACHE_DIR);
    let cache = cache_paths(&directory, key);
    // 缓存里存的是 RGBA（与模糊算法的输入一致），上屏前要换成 BGRA。
    let blurred = into_bgra(read_cache(&cache.blurred)?);
    // 换图后旧缓存没机会清：这里顺带收一次，目录最多留下这一对文件。
    prune_cache(&directory, &cache);
    Some((read_cache(&cache.source), blurred))
}

/// 清掉别的壁纸与同一张壁纸旧模糊半径留下的缓存，只保留 `keep` 这两份。
///
/// 每次换图或换模糊值都会调用；缓存目录于是稳定在一张壁纸两份文件，
/// 不会随着拖滑杆无限增长。
pub fn prune_cache(directory: &Path, keep: &CachePaths) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep.source || path == keep.blurred {
            continue;
        }
        let _ = std::fs::remove_file(path);
    }
}

/// 读图并缩放，返回 RGBA。GIF 取第一帧。
///
/// 这一步只取决于图片文件，可以跨模糊半径复用：4K 图解码加缩放要上百毫秒，
/// 拖模糊滑杆时没必要每次都重做。
pub fn load_source_rgba(key: &WallpaperKey) -> Result<image::RgbaImage, String> {
    let decoded = image::ImageReader::open(&key.path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok(downscale(decoded, key.max_edge))
}

/// 对已缩放的底图做模糊；`blur` 为 0 时直接复制。
pub fn blur_source(source: &image::RgbaImage, blur: u8) -> image::RgbaImage {
    if blur == 0 {
        source.clone()
    } else {
        gaussian_blur(source, blur as f32)
    }
}

/// 把最长边压到 `max_edge` 以内；小图原样返回。
pub fn downscale(image: image::RgbaImage, max_edge: u32) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if max_edge == 0 || longest <= max_edge {
        return image;
    }
    let scale = max_edge as f32 / longest as f32;
    let width = ((width as f32 * scale).round() as u32).max(1);
    let height = ((height as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
}

/// 高斯模糊，用三次盒式模糊近似（中心极限定理）。
///
/// 不用 `image::imageops::blur`：它的耗时与半径成正比，debug 构建下 1600px
/// 的壁纸配 30px 半径要几十秒，滑杆拖动时永远算不完。这里的滑动窗口与半径
/// 无关，单次遍历即可，观感几乎一致。
pub fn gaussian_blur(image: &image::RgbaImage, sigma: f32) -> image::RgbaImage {
    if sigma <= 0.0 {
        return image.clone();
    }
    // 三次盒式方差之和等于高斯方差：每个盒子的半宽由 sigma 反推。
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut radius = ((ideal - 1.0) / 2.0).round().max(0.0) as u32;
    if radius == 0 {
        radius = 1;
    }
    let mut output = image.clone();
    for _ in 0..3 {
        output = box_blur_horizontal(&output, radius);
        output = box_blur_vertical(&output, radius);
    }
    output
}

/// 水平方向的盒式模糊：每一行用滑动窗口累加，边缘按最近像素延展。
fn box_blur_horizontal(image: &image::RgbaImage, radius: u32) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    let mut output = image::RgbaImage::new(width, height);
    let window = (2 * radius + 1) as usize;
    for y in 0..height {
        let row: Vec<[u8; 4]> = (0..width).map(|x| image.get_pixel(x, y).0).collect();
        let mut sum = [0u32; 4];
        // 先装填窗口：左侧越界部分用第 0 个像素补齐。
        for index in 0..=radius {
            let pixel = row[index.min(width - 1) as usize];
            for channel in 0..4 {
                sum[channel] += pixel[channel] as u32;
            }
        }
        for channel in 0..4 {
            sum[channel] += row[0][channel] as u32 * radius;
        }
        for x in 0..width {
            let pixel = [
                (sum[0] / window as u32) as u8,
                (sum[1] / window as u32) as u8,
                (sum[2] / window as u32) as u8,
                (sum[3] / window as u32) as u8,
            ];
            output.put_pixel(x, y, image::Rgba(pixel));
            let leaving = row[x.saturating_sub(radius) as usize];
            let entering = row[(x + radius + 1).min(width - 1) as usize];
            for channel in 0..4 {
                sum[channel] = sum[channel] + entering[channel] as u32 - leaving[channel] as u32;
            }
        }
    }
    output
}

/// 垂直方向，逻辑与水平方向相同。
fn box_blur_vertical(image: &image::RgbaImage, radius: u32) -> image::RgbaImage {
    let (width, height) = image.dimensions();
    let mut output = image::RgbaImage::new(width, height);
    let window = (2 * radius + 1) as usize;
    for x in 0..width {
        let column: Vec<[u8; 4]> = (0..height).map(|y| image.get_pixel(x, y).0).collect();
        let mut sum = [0u32; 4];
        for index in 0..=radius {
            let pixel = column[index.min(height - 1) as usize];
            for channel in 0..4 {
                sum[channel] += pixel[channel] as u32;
            }
        }
        for channel in 0..4 {
            sum[channel] += column[0][channel] as u32 * radius;
        }
        for y in 0..height {
            let pixel = [
                (sum[0] / window as u32) as u8,
                (sum[1] / window as u32) as u8,
                (sum[2] / window as u32) as u8,
                (sum[3] / window as u32) as u8,
            ];
            output.put_pixel(x, y, image::Rgba(pixel));
            let leaving = column[y.saturating_sub(radius) as usize];
            let entering = column[(y + radius + 1).min(height - 1) as usize];
            for channel in 0..4 {
                sum[channel] = sum[channel] + entering[channel] as u32 - leaving[channel] as u32;
            }
        }
    }
    output
}

/// GPUI 的 `RenderImage` 按 BGRA 解读字节。
pub fn into_bgra(mut image: image::RgbaImage) -> image::RgbaImage {
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缩放 + 模糊的组合，只给测试用；应用走 `load_source_rgba` + `blur_source`。
    fn prepare(image: image::RgbaImage, blur: u8, max_edge: u32) -> image::RgbaImage {
        blur_source(&downscale(image, max_edge), blur)
    }

    #[test]
    fn resizes_the_longest_edge_and_keeps_the_ratio() {
        let image = image::RgbaImage::from_pixel(3200, 1200, image::Rgba([1, 2, 3, 255]));
        let prepared = prepare(image, 0, 1600);
        assert_eq!(prepared.dimensions(), (1600, 600));
        // 默认上限压到 1200：模糊后看不出细节，省一半像素。
        assert_eq!(downscale(prepared, MAX_EDGE).dimensions(), (1200, 450));
    }

    #[test]
    fn leaves_small_images_at_their_size() {
        let image = image::RgbaImage::from_pixel(320, 200, image::Rgba([1, 2, 3, 255]));
        let prepared = prepare(image, 10, 1600);
        assert_eq!(prepared.dimensions(), (320, 200));
    }

    #[test]
    fn blur_keeps_dimensions_and_softens_edges() {
        let mut image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
        // 8×8 的白色方块：能量够多，模糊后仍能观察到过渡。
        for y in 28..36 {
            for x in 28..36 {
                image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let prepared = prepare(image, 6, 1600);
        assert_eq!(prepared.dimensions(), (64, 64));
        // 方块边缘被晕开：外侧像素不再是纯黑，中心也不再是纯白。
        assert!(prepared.get_pixel(26, 32)[0] > 0);
        assert!(prepared.get_pixel(31, 31)[0] < 255);
    }

    #[test]
    fn blur_strength_grows_with_the_slider() {
        // 一条竖直的明暗分界：模糊越强，相邻像素的差越小。
        let mut image = image::RgbaImage::from_pixel(128, 128, image::Rgba([0, 0, 0, 255]));
        for y in 0..128 {
            for x in 64..128 {
                image.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        // 总梯度对阶跃是守恒的，看不出强弱；看分界处的最大坡度。
        let steepest = |prepared: &image::RgbaImage| {
            let mut steepest = 0f64;
            for y in 1..127 {
                for x in 1..127 {
                    let step = (prepared.get_pixel(x, y)[0] as f64
                        - prepared.get_pixel(x + 1, y)[0] as f64)
                        .abs();
                    steepest = steepest.max(step);
                }
            }
            steepest
        };
        let light = steepest(&prepare(image.clone(), 4, 0));
        let heavy = steepest(&prepare(image, 20, 0));
        assert!(light > 0.0 && light < 255.0, "小幅模糊也要产生过渡");
        assert!(heavy < light, "模糊滑杆加大后过渡应该更平缓");
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 40]));
        let swapped = into_bgra(image);
        assert_eq!(swapped.get_pixel(0, 0).0, [30, 20, 10, 40]);
    }

    /// 拖模糊滑杆只重做模糊：底图复用这段耗时决定手感。
    #[test]
    fn blurring_a_cached_source_is_cheap() {
        let the_source = image::RgbaImage::from_pixel(1200, 675, image::Rgba([90, 120, 180, 255]));
        let start = std::time::Instant::now();
        let blurred = blur_source(&the_source, 30);
        assert_eq!(blurred.dimensions(), (1200, 675));
        assert!(
            start.elapsed() < std::time::Duration::from_millis(600),
            "单次重模糊耗时 {:?}，拖杆会卡",
            start.elapsed()
        );
    }

    #[test]
    fn same_file_ignores_the_blur_radius() {
        let key = WallpaperKey {
            path: std::path::PathBuf::from("a.jpg"),
            modified: None,
            len: Some(10),
            blur: 4,
            max_edge: MAX_EDGE,
        };
        assert!(key.same_file(&WallpaperKey {
            blur: 30,
            ..key.clone()
        }));
        assert!(!key.same_file(&WallpaperKey {
            path: "b.jpg".into(),
            ..key.clone()
        }));
        assert!(!key.same_file(&WallpaperKey {
            max_edge: 1600,
            ..key.clone()
        }));
    }

    #[test]
    fn the_disk_cache_round_trips_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let key = WallpaperKey {
            path: std::path::PathBuf::from("a.jpg"),
            modified: None,
            len: Some(10),
            blur: 12,
            max_edge: MAX_EDGE,
        };
        let paths = cache_paths(directory.path(), &key);
        let mut image = image::RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 4]));
        image.put_pixel(2, 1, image::Rgba([9, 8, 7, 6]));
        write_cache(&paths.blurred, &image).unwrap();

        let read = read_cache(&paths.blurred).expect("命中所写缓存");
        assert_eq!(read.dimensions(), (4, 3));
        assert_eq!(read.get_pixel(2, 1).0, [9, 8, 7, 6]);
        assert_eq!(read.get_pixel(0, 0).0, [1, 2, 3, 4]);
    }

    #[test]
    fn a_truncated_or_missing_cache_counts_as_a_miss() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("none.bgra");
        assert!(read_cache(&missing).is_none());

        let broken = directory.path().join("broken.bgra");
        std::fs::write(&broken, [4u8, 0, 0, 0, 3, 0, 0, 0, 1, 2]).unwrap();
        assert!(read_cache(&broken).is_none(), "尺寸对不上时不能当命中");
    }

    #[test]
    fn one_change_of_blur_keeps_one_cache_pair() {
        let directory = tempfile::tempdir().unwrap();
        let mut key = WallpaperKey {
            path: std::path::PathBuf::from("a.jpg"),
            modified: None,
            len: Some(10),
            blur: 4,
            max_edge: MAX_EDGE,
        };
        let pixel = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 4]));
        for blur in [4, 10, 20] {
            key.blur = blur;
            let paths = cache_paths(directory.path(), &key);
            write_cache(&paths.source, &pixel).unwrap();
            write_cache(&paths.blurred, &pixel).unwrap();
            prune_cache(directory.path(), &paths);
        }
        let left: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left.len(), 2, "同一张图只留底图与当前成品：{left:?}");
    }
}
