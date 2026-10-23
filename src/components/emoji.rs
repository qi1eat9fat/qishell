//! 名字里的旗帜：`🇭🇰` 这类两个区域指示符换成真正的国旗图形。
//!
//! 不走字体是有原因的：GPUI 在 Windows 上把 emoji 交给 DirectWrite 的
//! COLR 彩色字形，而彩色字体（Twemoji、Segoe UI Emoji）的旗帜基字形都没有
//! 轮廓，光栅化边界为零，什么都画不出来；系统字体里区域指示符也只是两个
//! 字母。所以这里把旗子直接画成随包附带的 Twemoji 图片。

use gpui_kit::*;

use super::flags;

/// 一段名字：普通文字，或一面旗。
enum Piece {
    Text(String),
    Flag(&'static str),
}

/// 把 `text` 切成文字与国旗两种片段。两个相邻的区域指示符合成一面旗；
/// 找不到对应图形时留作普通文字。
fn pieces(text: &str) -> Vec<Piece> {
    let characters: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if let Some(code) = country_code(&characters, index) {
            if !buffer.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut buffer)));
            }
            pieces.push(Piece::Flag(code));
            index += 2;
            continue;
        }
        buffer.push(character);
        index += 1;
    }
    if !buffer.is_empty() {
        pieces.push(Piece::Text(buffer));
    }
    pieces
}

/// `characters[index..]` 处的两个区域指示符对应的国家代码。
fn country_code(characters: &[char], index: usize) -> Option<&'static str> {
    let first = regional_indicator(characters.get(index).copied()?)?;
    let second = regional_indicator(characters.get(index + 1).copied()?)?;
    let code: String = [first, second].iter().collect();
    flags::CODES.iter().find(|known| **known == code).copied()
}

/// U+1F1E6 到 U+1F1FF 是 A 到 Z 的区域指示符。
fn regional_indicator(character: char) -> Option<char> {
    matches!(character, '\u{1F1E6}'..='\u{1F1FF}')
        .then(|| char::from_u32('A' as u32 + (character as u32 - 0x1F1E6)))
        .flatten()
}

/// `text` 里所有国旗的两位代码，按出现顺序。
pub fn flag_codes(text: &str) -> Vec<&'static str> {
    pieces(text)
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Flag(code) => Some(code),
            Piece::Text(_) => None,
        })
        .collect()
}

/// 把 `text` 开头的连续国旗与其余文字分开：`("🇭🇰", "203.0.113.10")`。
///
/// 输入框画不了国旗，名称输入框只能把开头的旗从显示文字里拿掉、另画成图片，
/// 保存时再接回去。旗不在开头时原样留在文字里：硬把它挪到最前面会改掉用户
/// 写下的名字。
pub fn take_leading_flags(text: &str) -> (String, String) {
    let characters: Vec<char> = text.chars().collect();
    let mut flags = String::new();
    let mut index = 0;
    while let Some(code) = country_code(&characters, index) {
        flags.push_str(&flag_emoji(code));
        index += 2;
    }
    (flags, characters[index..].iter().collect())
}

/// 两位代码拼回国旗 emoji（`HK` → `🇭🇰`）。
fn flag_emoji(code: &str) -> String {
    code.chars()
        .map(|letter| {
            (letter as u32)
                .checked_sub('A' as u32)
                .and_then(|offset| char::from_u32(0x1F1E6 + offset))
                .unwrap_or(letter)
        })
        .collect()
}

/// 单独一面旗，给定尺寸。代码未知或解不出图时为 `None`。
pub fn flag(code: &str, size: Pixels) -> Option<AnyElement> {
    let image = flags::flag_image(code)?;
    Some(
        div()
            .flex_shrink_0()
            .child(
                img(image)
                    .size(size)
                    .rounded(px(1.))
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element(),
    )
}

/// `text` 里的国旗画成图形，其余仍是文字。`size` 是行高，旗子按它定尺寸。
pub fn with_flags(text: impl Into<SharedString>, size: Pixels) -> AnyElement {
    let text = text.into();
    let pieces = pieces(&text);
    // 没有旗子就原样返回文字，不额外套一层 flex——调用方的 `truncate()`
    // 才能直接作用在这段文字上。
    if pieces.len() == 1 && matches!(pieces.first(), Some(Piece::Text(_))) {
        return text.into_any_element();
    }
    let mut row = div().flex().min_w_0().items_center();
    for piece in pieces {
        row = row.child(match piece {
            // 文字段可以压缩：父层宽度不够时由它出省略号，旗子始终保持完整。
            Piece::Text(run) => div().min_w_0().truncate().child(run).into_any_element(),
            Piece::Flag(code) => {
                // 图片走 `img` 元素：`svg` 元素画的是单色蒙版，彩旗会被染成
                // 一个色块。图片按行高定尺寸，右边留 4px 与后面的字分开；
                // 下移 1px 让旗面与数字的视觉中心对齐（数字的字身比行高
                // 低一点，居中摆放时旗子看起来偏高）。
                match flags::flag_image(code) {
                    Some(image) => div()
                        .flex_shrink_0()
                        .mr(px(4.))
                        .relative()
                        .top(px(1.))
                        .child(
                            img(image)
                                .size(size)
                                .rounded(px(1.))
                                .object_fit(ObjectFit::Cover),
                        )
                        .into_any_element(),
                    // 解不出图就退回代码本身，总比什么都不显示强。
                    None => div()
                        .flex_shrink_0()
                        .child(code.to_string())
                        .into_any_element(),
                }
            }
        });
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    // 只导入被测函数：`use super::*` 会把 `gpui_kit::*` 再展开一遍，
    // 编译器的 glob 展开在递归上限下会爆栈。
    use super::{Piece, pieces, take_leading_flags};

    #[test]
    fn a_flag_is_one_piece_of_its_own() {
        let result = pieces("\u{1F1ED}\u{1F1F0}Hong Kong");
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], Piece::Flag("HK")));
        assert!(matches!(&result[1], Piece::Text(text) if text == "Hong Kong"));
    }

    #[test]
    fn plain_text_stays_one_piece() {
        let result = pieces("\u{751F}\u{4EA7}\u{673A}");
        assert_eq!(result.len(), 1);
        assert!(matches!(&result[0], Piece::Text(text) if text == "\u{751F}\u{4EA7}\u{673A}"));
    }

    #[test]
    fn an_unknown_pair_stays_text() {
        // AA 不是国家代码，两个指示符原样留在文字里。
        let result = pieces("\u{1F1E6}\u{1F1E6}");
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Piece::Text(_)));
    }

    #[test]
    fn leading_flags_come_off_the_text() {
        // 开头的旗摘出来，剩下的才是输入框里显示的文字。
        let (flags, rest) = take_leading_flags("\u{1F1ED}\u{1F1F0}203.0.113.10");
        assert_eq!(flags, "\u{1F1ED}\u{1F1F0}");
        assert_eq!(rest, "203.0.113.10");
    }

    #[test]
    fn flags_after_the_first_word_stay_put() {
        // 旗不在开头就原样留在文字里：挪到最前面会改掉用户写下的名字。
        let (flags, rest) = take_leading_flags("Hong Kong \u{1F1ED}\u{1F1F0}");
        assert_eq!(flags, "");
        assert_eq!(rest, "Hong Kong \u{1F1ED}\u{1F1F0}");
    }

    #[test]
    fn a_plain_name_is_left_alone() {
        let (flags, rest) = take_leading_flags("\u{751F}\u{4EA7}\u{673A}");
        assert_eq!(flags, "");
        assert_eq!(rest, "\u{751F}\u{4EA7}\u{673A}");
    }
}
