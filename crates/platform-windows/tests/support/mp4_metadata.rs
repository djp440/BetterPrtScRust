//! 独立读取 MP4，并用已有 AV1 解析器检查编码尺寸、渲染尺寸和像素比例。

use std::path::Path;

fn number(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut result = Vec::new();
    while !data.is_empty() {
        assert!(data.len() >= 8, "截断的 MP4 box");
        let kind = data[4..8].try_into().unwrap();
        let (length, header) = match number(&data[..4]) {
            0 => (data.len(), 8),
            1 => {
                assert!(data.len() >= 16);
                (
                    usize::try_from(u64::from_be_bytes(data[8..16].try_into().unwrap())).unwrap(),
                    16,
                )
            }
            length => (length as usize, 8),
        };
        assert!(
            length >= header && length <= data.len(),
            "无效的 MP4 box 长度"
        );
        result.push((kind, &data[header..length]));
        data = &data[length..];
    }
    result
}

fn child<'a>(data: &'a [u8], name: &[u8; 4]) -> &'a [u8] {
    boxes(data)
        .into_iter()
        .find(|(kind, _)| kind == name)
        .expect("缺少 MP4 box")
        .1
}

pub fn assert_av1_metadata(path: &Path, render_width: u32, render_height: u32) {
    let bytes = std::fs::read(path).unwrap();
    let movie = child(&bytes, b"moov");
    let mut found = 0;
    for (_, track) in boxes(movie).into_iter().filter(|(kind, _)| kind == b"trak") {
        let table = child(child(child(track, b"mdia"), b"minf"), b"stbl");
        let description = child(table, b"stsd");
        let entries = boxes(&description[8..]);
        assert_eq!(entries.len(), number(&description[4..8]) as usize);
        for (_, entry) in entries.into_iter().filter(|(kind, _)| kind == b"av01") {
            found += 1;
            assert!(entry.len() >= 78);
            let width = u32::from(u16::from_be_bytes(entry[24..26].try_into().unwrap()));
            let height = u32::from(u16::from_be_bytes(entry[26..28].try_into().unwrap()));
            let config = child(&entry[78..], b"av1C");
            assert!(config.len() >= 4);
            assert_eq!(config[0], 0x81);
            let header = avif_parse::AV1Metadata::parse_av1_bitstream(&config[4..]).unwrap();
            assert_eq!(
                (width, height),
                (header.max_frame_width.get(), header.max_frame_height.get())
            );
            assert!(render_width <= width && render_height <= height);
            assert_eq!(config[1] >> 5, header.seq_profile);
            assert_eq!(config[2] & 0x40 != 0, header.bit_depth > 8);
            assert_eq!(config[2] & 0x20 != 0, header.bit_depth == 12);
            assert_eq!(config[2] & 0x10 != 0, header.monochrome);
            assert_eq!(
                (config[2] & 8 != 0, config[2] & 4 != 0),
                header.chroma_subsampling
            );
            if (width, height) != (render_width, render_height) {
                let aspect = child(&entry[78..], b"pasp");
                let horizontal = u128::from(number(&aspect[..4]));
                let vertical = u128::from(number(&aspect[4..8]));
                assert!(horizontal > 0 && vertical > 0);
                assert_eq!(
                    horizontal * u128::from(width) * u128::from(render_height),
                    vertical * u128::from(render_width) * u128::from(height)
                );
            }
            let track_header = child(track, b"tkhd");
            let end = track_header.len();
            assert_eq!(number(&track_header[end - 8..end - 4]), render_width << 16);
            assert_eq!(number(&track_header[end - 4..]), render_height << 16);
        }
    }
    assert_eq!(found, 1, "应恰有一条 AV1 视频轨道");
}
