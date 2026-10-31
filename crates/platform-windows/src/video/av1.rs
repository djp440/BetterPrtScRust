//! AV1 的 MP4 sample description。Windows 10 的 MPEG-4 sink 不会自动生成它。
//!
//! 格式依据 AOM 的 AV1 ISOBMFF 规范（AV1SampleEntry / AV1CodecConfigurationRecord）：
//! https://aomediacodec.github.io/av1-isobmff/
//! 只读取 sequence header 中构造 av1C 所需的字段，不解析或改写视频帧。

use bps_platform::PlatformError;

/// 完整的 stsd box。av01 尺寸来自序列头；render 尺寸只用于保持显示宽高比。
pub(super) fn sample_description(
    header: &[u8],
    render_width: u32,
    render_height: u32,
) -> Result<Vec<u8>, PlatformError> {
    let invalid = || PlatformError::new("AV1 序列头无效，无法生成 MP4 格式描述");
    let render_width = u16::try_from(render_width)
        .ok()
        .filter(|&n| n != 0)
        .ok_or_else(invalid)?;
    let render_height = u16::try_from(render_height)
        .ok()
        .filter(|&n| n != 0)
        .ok_or_else(invalid)?;
    let (obu, payload) = sequence_header(header).ok_or_else(invalid)?;
    let config = configuration(payload).ok_or_else(invalid)?;
    if render_width > config.width || render_height > config.height {
        return Err(invalid());
    }

    // ISO/IEC 14496-12 VisualSampleEntry：包含 8 字节 box header，共 86 字节。
    let mut entry = vec![0; 86];
    entry[4..8].copy_from_slice(b"av01");
    entry[14..16].copy_from_slice(&1u16.to_be_bytes()); // data_reference_index
    entry[32..34].copy_from_slice(&config.width.to_be_bytes());
    entry[34..36].copy_from_slice(&config.height.to_be_bytes());
    entry[36..40].copy_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
    entry[40..44].copy_from_slice(&0x0048_0000u32.to_be_bytes());
    entry[48..50].copy_from_slice(&1u16.to_be_bytes()); // frame_count
    entry[82..84].copy_from_slice(&24u16.to_be_bytes()); // depth
    entry[84..86].copy_from_slice(&u16::MAX.to_be_bytes()); // pre_defined = -1

    let config_size = u32::try_from(12 + obu.len()).map_err(|_| invalid())?;
    entry.extend_from_slice(&config_size.to_be_bytes());
    entry.extend_from_slice(b"av1C");
    entry.extend_from_slice(&config.bytes);
    entry.extend_from_slice(obu);
    if (render_width, render_height) != (config.width, config.height) {
        // AV1 ISOBMFF §2.2.4：pasp 补偿最大编码尺寸与实际 render 尺寸的比例。
        let horizontal = u32::from(render_width) * u32::from(config.height);
        let vertical = u32::from(config.width) * u32::from(render_height);
        let (mut divisor, mut remainder) = (horizontal, vertical);
        while remainder != 0 {
            (divisor, remainder) = (remainder, divisor % remainder);
        }
        entry.extend_from_slice(&16u32.to_be_bytes());
        entry.extend_from_slice(b"pasp");
        entry.extend_from_slice(&(horizontal / divisor).to_be_bytes());
        entry.extend_from_slice(&(vertical / divisor).to_be_bytes());
    }
    let entry_size = u32::try_from(entry.len()).map_err(|_| invalid())?;
    entry[..4].copy_from_slice(&entry_size.to_be_bytes());

    let mut stsd = Vec::with_capacity(16 + entry.len());
    stsd.extend_from_slice(
        &entry_size
            .checked_add(16)
            .ok_or_else(invalid)?
            .to_be_bytes(),
    );
    stsd.extend_from_slice(b"stsd");
    stsd.extend_from_slice(&0u32.to_be_bytes()); // version / flags
    stsd.extend_from_slice(&1u32.to_be_bytes()); // entry_count
    stsd.extend_from_slice(&entry);
    Ok(stsd)
}

/// 读取带长度字段的 OBU，跳过时间分隔符、元数据等，保留完整的 sequence header OBU。
fn sequence_header(mut data: &[u8]) -> Option<(&[u8], &[u8])> {
    while !data.is_empty() {
        let original = data;
        let (&header, rest) = data.split_first()?;
        // forbidden / reserved 位必须为零；av1C 内的 OBU 必须带 size field。
        if header & 0x83 != 0x02 {
            return None;
        }
        data = rest;
        if header & 0x04 != 0 {
            let (&extension, rest) = data.split_first()?;
            if extension & 7 != 0 {
                return None;
            }
            data = rest;
        }
        let mut length = 0u64;
        for index in 0..8 {
            let (&byte, rest) = data.split_first()?;
            data = rest;
            length |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                let length = usize::try_from(u32::try_from(length).ok()?).ok()?;
                let payload = data.get(..length)?;
                data = data.get(length..)?;
                if (header >> 3) & 15 == 1 {
                    return Some((&original[..original.len() - data.len()], payload));
                }
                break;
            }
            if index == 7 {
                return None;
            }
        }
    }
    None
}

/// AV1 sequence_header_obu / color_config，位序遵循 AV1 规范 §5.5.1、§5.5.2。
struct Configuration {
    bytes: [u8; 4],
    width: u16,
    height: u16,
}

fn configuration(payload: &[u8]) -> Option<Configuration> {
    let mut bits = Bits {
        data: payload,
        offset: 0,
    };
    let profile = bits.read(3)? as u8;
    if profile > 2 {
        return None;
    }
    let still = bits.flag()?;
    let reduced = bits.flag()?;
    if reduced && !still {
        return None;
    }
    let (mut level, mut tier) = (0, false);
    if reduced {
        level = bits.read(5)? as u8;
    } else {
        let mut delay_bits = None;
        if bits.flag()? {
            // timing_info_present_flag
            bits.skip(64)?;
            if bits.flag()? {
                // equal_picture_interval
                bits.uvlc()?;
            }
            if bits.flag()? {
                // decoder_model_info_present_flag
                delay_bits = Some(bits.read(5)? as usize + 1);
                bits.skip(42)?;
            }
        }
        let display_delay = bits.flag()?;
        let operating_points = bits.read(5)? + 1;
        for index in 0..operating_points {
            bits.skip(12)?;
            let current_level = bits.read(5)? as u8;
            let current_tier = current_level > 7 && bits.flag()?;
            if index == 0 {
                (level, tier) = (current_level, current_tier);
            }
            if let Some(delay_bits) = delay_bits {
                if bits.flag()? {
                    bits.skip(delay_bits * 2 + 1)?;
                }
            }
            if display_delay && bits.flag()? {
                bits.skip(4)?;
            }
        }
    }
    let width_bits = bits.read(4)? as usize + 1;
    let height_bits = bits.read(4)? as usize + 1;
    let width = u16::try_from(bits.read(width_bits)? + 1).ok()?;
    let height = u16::try_from(bits.read(height_bits)? + 1).ok()?;
    if !reduced && bits.flag()? {
        // frame_id_numbers_present_flag
        bits.skip(7)?;
    }
    bits.skip(3)?; // superblock / filter_intra / intra_edge_filter
    if !reduced {
        bits.skip(4)?; // interintra / masked_compound / warped_motion / dual_filter
        let order_hint = bits.flag()?;
        if order_hint {
            bits.skip(2)?;
        }
        let screen_content_tools = if bits.flag()? { 2 } else { bits.read(1)? };
        if screen_content_tools > 0 && !bits.flag()? {
            bits.skip(1)?;
        }
        if order_hint {
            bits.skip(3)?;
        }
    }
    bits.skip(3)?; // superres / cdef / restoration
    let high = bits.flag()?;
    let twelve = profile == 2 && high && bits.flag()?;
    let monochrome = profile != 1 && bits.flag()?;
    let (primaries, transfer, matrix) = if bits.flag()? {
        (bits.read(8)?, bits.read(8)?, bits.read(8)?)
    } else {
        (2, 2, 2)
    };
    let (sub_x, sub_y, position) = if monochrome {
        bits.skip(1)?; // color_range
        (true, true, 0)
    } else {
        let (x, y) = if (primaries, transfer, matrix) == (1, 13, 0) {
            (false, false)
        } else {
            bits.skip(1)?; // color_range
            match profile {
                0 => (true, true),
                1 => (false, false),
                _ if twelve => {
                    let x = bits.flag()?;
                    (x, x && bits.flag()?)
                }
                _ => (true, false),
            }
        };
        let position = if x && y { bits.read(2)? as u8 } else { 0 };
        bits.skip(1)?; // separate_uv_delta_q
        (x, y, position)
    };
    bits.skip(1)?; // film_grain_params_present
    if !bits.flag()? {
        // trailing_one_bit
        return None;
    }
    while bits.offset < payload.len() * 8 {
        if bits.flag()? {
            return None;
        }
    }
    Some(Configuration {
        width,
        height,
        bytes: [
            0x81, // marker = 1, version = 1
            (profile << 5) | level,
            (u8::from(tier) << 7)
                | (u8::from(high) << 6)
                | (u8::from(twelve) << 5)
                | (u8::from(monochrome) << 4)
                | (u8::from(sub_x) << 3)
                | (u8::from(sub_y) << 2)
                | position,
            0, // 没有额外的 initial_presentation_delay 约束。
        ],
    })
}

struct Bits<'a> {
    data: &'a [u8],
    offset: usize,
}

impl Bits<'_> {
    fn read(&mut self, count: usize) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = *self.data.get(self.offset / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - self.offset % 8)) & 1);
            self.offset += 1;
        }
        Some(value)
    }

    fn flag(&mut self) -> Option<bool> {
        Some(self.read(1)? != 0)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.offset = self.offset.checked_add(count)?;
        (self.offset <= self.data.len() * 8).then_some(())
    }

    fn uvlc(&mut self) -> Option<()> {
        for zeros in 0..32 {
            if self.flag()? {
                return self.skip(zeros);
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RTX 5070 Ti、256×256、30 fps 的真实 sequence header；不含桌面内容。
    const HEADER: &[u8] = &[
        0x0a, 0x0c, 0x00, 0x00, 0x00, 0x42, 0xab, 0xbf, 0xc3, 0xfe, 0x00, 0x10, 0xcc, 0x02,
    ];

    #[test]
    fn real_encoder_header_produces_av01_with_matching_av1c() {
        let description = sample_description(HEADER, 256, 256).unwrap();
        assert_eq!(
            u32::from_be_bytes(description[..4].try_into().unwrap()) as usize,
            description.len()
        );
        assert_eq!(&description[4..8], b"stsd");
        assert_eq!(&description[12..16], &1u32.to_be_bytes());
        assert_eq!(&description[20..24], b"av01");
        assert_eq!(&description[48..52], &[0x07, 0x80, 0x04, 0x40]);
        assert_eq!(&description[106..110], b"av1C");
        assert_eq!(&description[110..114], &[0x81, 8, 0x0c, 0]);
        assert_eq!(&description[114..114 + HEADER.len()], HEADER);
    }

    #[test]
    fn sample_entry_uses_sequence_maximum_dimensions() {
        // 同一序列可包含 256×256 小帧；av01 必须描述序列最大尺寸 1920×1088。
        let description = sample_description(HEADER, 256, 256).unwrap();
        let header = avif_parse::AV1Metadata::parse_av1_bitstream(HEADER).unwrap();
        assert_eq!(
            &description[48..50],
            &(header.max_frame_width.get() as u16).to_be_bytes()
        );
        assert_eq!(
            &description[50..52],
            &(header.max_frame_height.get() as u16).to_be_bytes()
        );
    }

    #[test]
    fn sample_entry_keeps_the_render_aspect_ratio_and_checks_bounds() {
        let description = sample_description(HEADER, 384, 256).unwrap();
        let pasp = &description[description.len() - 16..];
        assert_eq!(&pasp[..8], &[0, 0, 0, 16, b'p', b'a', b's', b'p']);
        assert_eq!(&pasp[8..12], &17u32.to_be_bytes());
        assert_eq!(&pasp[12..16], &20u32.to_be_bytes());
        let maximum = sample_description(HEADER, 1920, 1088).unwrap();
        assert_eq!(&maximum[114..], HEADER);
        assert!(sample_description(HEADER, 1921, 1088).is_err());
        assert!(sample_description(HEADER, 1920, 1089).is_err());
    }

    #[test]
    fn truncated_and_invalid_headers_return_errors() {
        for length in 0..HEADER.len() {
            assert!(
                sample_description(&HEADER[..length], 256, 256).is_err(),
                "截断到 {length} 字节"
            );
        }
        for header in [
            vec![0x8a, 0],
            vec![0x0a, 0xff],
            vec![0x0a; 12],
            vec![0x08, 0],
            vec![0x12, 0],
        ] {
            assert!(sample_description(&header, 256, 256).is_err());
        }
        assert!(sample_description(HEADER, 0, 256).is_err());
        assert!(sample_description(HEADER, 256, 65_536).is_err());
    }

    #[test]
    fn leading_temporal_delimiter_is_not_copied_to_configuration_obus() {
        let mut header = vec![0x12, 0];
        header.extend_from_slice(HEADER);
        assert_eq!(
            sample_description(&header, 384, 256).unwrap(),
            sample_description(HEADER, 384, 256).unwrap()
        );
    }

    #[test]
    fn extension_and_multibyte_obu_length_are_preserved() {
        let mut header = vec![0x0e, 0, 0x8c, 0];
        header.extend_from_slice(&HEADER[2..]);
        let description = sample_description(&header, 256, 256).unwrap();
        assert_eq!(&description[110..114], &[0x81, 8, 0x0c, 0]);
        assert_eq!(&description[114..114 + header.len()], &header);
        header[1] = 1; // extension 的保留位不能置一。
        assert!(sample_description(&header, 256, 256).is_err());
    }
}
