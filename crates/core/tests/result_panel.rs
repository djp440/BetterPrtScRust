//! 结果面板：在本机识别截图里的文字（OCR），显示可编辑的识别结果。
//!
//! 面板在子进程里运行，识别在后台线程进行：[`ResultPanel`] 给出要做的识别（[`Recognition`]），
//! 界面在后台执行它，再把结果交回面板。测试里由假的文字识别同步完成这一步。

use bps_core::{Recognition, RecognitionStatus, ResultPanel};
use bps_platform::{Image, TextRecognizer};
use bps_platform_fake::FakePlatform;

fn screenshot() -> Image {
    Image::from_rgba(4, 2, (0..32).collect())
}

/// 系统装有中文和英文 OCR 语言包、默认识别语言是中文的假平台。
fn chinese_and_english() -> FakePlatform {
    FakePlatform::new()
        .with_recognition_language("en-US", "English (United States)")
        .with_recognition_language("zh-Hans-CN", "中文(中华人民共和国)")
        .with_default_recognition_language("zh-Hans-CN")
}

/// 像界面的后台线程那样执行识别，把结果交回面板。
fn complete(panel: &mut ResultPanel, recognition: Recognition, recognizer: &dyn TextRecognizer) {
    panel.finish(recognition.run(recognizer));
}

#[test]
fn result_panel_recognizes_screenshot_with_default_recognition_language() {
    let fake = chinese_and_english()
        .with_recognized_text("zh-Hans-CN", &[&["Hello", "World"], &["第二行"]]);
    let recognizer = fake.text_recognizer();

    let (mut panel, recognition) = ResultPanel::open(screenshot(), recognizer.as_ref());

    assert_eq!(panel.status(), &RecognitionStatus::Recognizing);
    assert_eq!(panel.language(), Some("zh-Hans-CN"));
    complete(&mut panel, recognition.unwrap(), recognizer.as_ref());
    assert_eq!(panel.status(), &RecognitionStatus::Recognized);
    assert_eq!(panel.text(), "Hello World\n第二行");
    assert_eq!(
        fake.recognition_requests(),
        [(screenshot(), "zh-Hans-CN".to_owned())]
    );
    let languages: Vec<_> = panel.languages().iter().map(|l| l.tag.as_str()).collect();
    assert_eq!(languages, ["en-US", "zh-Hans-CN"]);
}

/// 打开结果面板并完成第一次识别。
fn open_and_recognize(fake: &FakePlatform) -> ResultPanel {
    let recognizer = fake.text_recognizer();
    let (mut panel, recognition) = ResultPanel::open(screenshot(), recognizer.as_ref());
    if let Some(recognition) = recognition {
        complete(&mut panel, recognition, recognizer.as_ref());
    }
    panel
}

#[test]
fn chinese_and_japanese_are_joined_without_spaces_while_other_words_keep_them() {
    let fake = chinese_and_english().with_recognized_text(
        "zh-Hans-CN",
        &[
            &[
                "使用", "Windows", "识", "别", "文", "字", "，", "支", "持", "OCR", "2.0", "版",
                "本",
            ],
            &["こんにちは", "世界", "（", "Hello", "World", "）"],
            &["안녕하세요", "세계"],
        ],
    );

    let panel = open_and_recognize(&fake);

    assert_eq!(
        panel.text(),
        "使用Windows识别文字，支持OCR 2.0版本\nこんにちは世界（Hello World）\n안녕하세요 세계"
    );
}

#[test]
fn without_any_ocr_language_pack_the_panel_says_so_instead_of_recognizing() {
    let fake = FakePlatform::new();
    let recognizer = fake.text_recognizer();

    let (panel, recognition) = ResultPanel::open(screenshot(), recognizer.as_ref());

    assert!(recognition.is_none());
    assert_eq!(panel.status(), &RecognitionStatus::NoLanguagePack);
    assert_eq!(panel.language(), None);
    assert!(panel.languages().is_empty());
    assert_eq!(panel.text(), "");
    assert!(fake.recognition_requests().is_empty());
}

#[test]
fn recognizing_again_after_installing_a_language_pack_uses_it() {
    let fake = FakePlatform::new().with_recognized_text("en-US", &[&["Installed"]]);
    let recognizer = fake.text_recognizer();
    let (mut panel, _) = ResultPanel::open(screenshot(), recognizer.as_ref());

    fake.install_recognition_language("en-US", "English (United States)");
    let recognition = panel.recognize_again(recognizer.as_ref()).unwrap();
    complete(&mut panel, recognition, recognizer.as_ref());

    assert_eq!(panel.language(), Some("en-US"));
    assert_eq!(panel.status(), &RecognitionStatus::Recognized);
    assert_eq!(panel.text(), "Installed");
}

#[test]
fn choosing_another_recognition_language_recognizes_the_screenshot_again() {
    let fake = chinese_and_english()
        .with_recognized_text("zh-Hans-CN", &[&["中", "文"]])
        .with_recognized_text("en-US", &[&["English"]]);
    let recognizer = fake.text_recognizer();
    let mut panel = open_and_recognize(&fake);

    let recognition = panel.select_language("en-US").unwrap();

    assert_eq!(panel.language(), Some("en-US"));
    assert_eq!(panel.status(), &RecognitionStatus::Recognizing);
    complete(&mut panel, recognition, recognizer.as_ref());
    assert_eq!(panel.text(), "English");
    assert_eq!(
        fake.recognition_requests(),
        [
            (screenshot(), "zh-Hans-CN".to_owned()),
            (screenshot(), "en-US".to_owned())
        ]
    );
    assert!(
        panel.select_language("en-US").is_none(),
        "选的就是当前识别语言时不用重新识别"
    );
}

#[test]
fn result_of_an_earlier_recognition_language_arriving_late_is_ignored() {
    let fake = chinese_and_english()
        .with_recognized_text("zh-Hans-CN", &[&["中", "文"]])
        .with_recognized_text("en-US", &[&["English"]]);
    let recognizer = fake.text_recognizer();
    let (mut panel, chinese) = ResultPanel::open(screenshot(), recognizer.as_ref());

    // 中文还没识别完，用户就换成了英文；英文先完成，中文的结果后到。
    let english = panel.select_language("en-US").unwrap();
    complete(&mut panel, english, recognizer.as_ref());
    complete(&mut panel, chinese.unwrap(), recognizer.as_ref());

    assert_eq!(panel.language(), Some("en-US"));
    assert_eq!(panel.status(), &RecognitionStatus::Recognized);
    assert_eq!(panel.text(), "English");
}

#[test]
fn failed_recognition_is_reported_and_can_be_retried() {
    let fake = chinese_and_english()
        .with_recognized_text("zh-Hans-CN", &[&["好", "了"]])
        .with_text_recognition_failing("识别引擎出错");
    let recognizer = fake.text_recognizer();
    let mut panel = open_and_recognize(&fake);

    assert_eq!(
        panel.status(),
        &RecognitionStatus::Failed("识别引擎出错".to_owned())
    );
    assert_eq!(panel.text(), "");

    fake.set_text_recognition_failing(None);
    let recognition = panel.recognize_again(recognizer.as_ref()).unwrap();
    complete(&mut panel, recognition, recognizer.as_ref());

    assert_eq!(panel.status(), &RecognitionStatus::Recognized);
    assert_eq!(panel.text(), "好了");
}

#[test]
fn recognized_text_can_be_edited() {
    let fake = chinese_and_english().with_recognized_text("zh-Hans-CN", &[&["识", "别"]]);
    let mut panel = open_and_recognize(&fake);

    panel.text_mut().push_str("结果");

    assert_eq!(panel.text(), "识别结果");
}

#[test]
fn full_width_punctuation_inside_numbers_and_latin_words_becomes_half_width() {
    // Windows 的中文 OCR 把“2.0”识别成“2”“．”“0”三个词。
    let fake = chinese_and_english().with_recognized_text(
        "zh-Hans-CN",
        &[
            &["版", "本", "2", "．", "0"],
            &["Version", "2", "．", "0", "，", "好", "。"],
        ],
    );

    let panel = open_and_recognize(&fake);

    assert_eq!(panel.text(), "版本2.0\nVersion 2.0，好。");
}

#[test]
fn middle_dot_takes_no_spaces_around_it() {
    // Windows 的中文 OCR 也会把“2.0”识别成“2”“·”“0”；间隔号本身（如外国人名）两边也不加空格。
    let fake = chinese_and_english().with_recognized_text(
        "zh-Hans-CN",
        &[&["版", "本", "2", "·", "0"], &["Michael", "·", "Jackson"]],
    );

    let panel = open_and_recognize(&fake);

    assert_eq!(panel.text(), "版本2·0\nMichael·Jackson");
}
