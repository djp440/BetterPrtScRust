//! 翻译：在结果面板里把识别出的文字交给用户配置的翻译服务（OpenAI 兼容接口或 DeepL）。
//!
//! 翻译在后台线程进行：[`ResultPanel`] 给出要做的 [`Translation`]，界面在后台执行它，
//! 再把结果交回面板。测试里由假的 HTTP 传输同步完成这一步，并检查发出的请求
//! （ADR 0004：只发送文字，截图本身绝不上传）。

use bps_core::{Config, Locale, ResultPanel, TranslationStatus};
use bps_platform::Image;
use bps_platform_fake::FakePlatform;

const KEY: &str = "sk-secret-key-123456";

const OPENAI: &str = r#"
[translation]
provider = "openai"
api_key = "sk-secret-key-123456"
model = "gpt-4o-mini"
target_language = "en"
"#;

const DEEPL: &str = r#"
[translation]
provider = "deepl"
api_key = "abc-123:fx"
target_language = "en"
"#;

/// 能识别出 “你好，世界” 的假平台。
fn fake() -> FakePlatform {
    FakePlatform::new()
        .with_recognition_language("zh-Hans-CN", "中文")
        .with_default_recognition_language("zh-Hans-CN")
        .with_recognized_text("zh-Hans-CN", &[&["你好，世界"]])
}

/// 已识别出文字的结果面板，翻译服务取自配置文件 `config_text`（可以没有），界面语言为 `locale`。
fn panel_with(fake: &FakePlatform, config_text: &str, locale: Locale) -> ResultPanel {
    let recognizer = fake.text_recognizer();
    let (mut panel, recognition) = ResultPanel::open(
        Image::from_rgba(4, 2, (0..32).collect()),
        recognizer.as_ref(),
    );
    panel.finish(recognition.unwrap().run(recognizer.as_ref()));
    let config = Config::parse(config_text).unwrap();
    panel.set_translation(config.translation, locale);
    panel
}

/// 像界面的后台线程那样执行翻译，把结果交回面板。
fn translate(panel: &mut ResultPanel, fake: &FakePlatform) {
    let translation = panel.start_translation().expect("应当可以翻译");
    panel.finish_translation(translation.run(fake.http_transport().as_ref()));
}

fn json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

#[test]
fn openai_compatible_service_translates_the_recognized_text() {
    let fake = fake().with_http_reply(
        200,
        r#"{"choices":[{"message":{"role":"assistant","content":"Hello, world"}}]}"#,
    );
    let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);

    translate(&mut panel, &fake);

    assert_eq!(
        panel.translation_status(),
        &TranslationStatus::Translated("Hello, world".to_owned())
    );
    let requests = fake.http_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, "https://api.openai.com/v1/chat/completions");
    assert!(
        request
            .headers
            .contains(&("Authorization".to_owned(), format!("Bearer {KEY}")))
    );
    let body = json(&request.body);
    assert_eq!(body["model"], "gpt-4o-mini");
    let sent = body.to_string();
    assert!(sent.contains("你好，世界"));
    assert!(sent.contains("English"));
}

#[test]
fn deepl_translates_the_recognized_text() {
    let fake = fake().with_http_reply(
        200,
        r#"{"translations":[{"detected_source_language":"ZH","text":"Hello, world"}]}"#,
    );
    let mut panel = panel_with(&fake, DEEPL, Locale::ZhCn);

    translate(&mut panel, &fake);

    assert_eq!(
        panel.translation_status(),
        &TranslationStatus::Translated("Hello, world".to_owned())
    );
    let request = &fake.http_requests()[0];
    // 免费版密钥（以 :fx 结尾）用免费版的地址。
    assert_eq!(request.url, "https://api-free.deepl.com/v2/translate");
    assert!(request.headers.contains(&(
        "Authorization".to_owned(),
        "DeepL-Auth-Key abc-123:fx".to_owned()
    )));
    let body = json(&request.body);
    assert_eq!(body["text"], serde_json::json!(["你好，世界"]));
    assert_eq!(body["target_lang"], "EN-US");
}

#[test]
fn deepl_pro_keys_use_the_pro_endpoint() {
    let fake = fake().with_http_reply(200, r#"{"translations":[{"text":"x"}]}"#);
    let mut panel = panel_with(
        &fake,
        "[translation]\nprovider = \"deepl\"\napi_key = \"abc-123\"\n",
        Locale::ZhCn,
    );
    translate(&mut panel, &fake);
    assert_eq!(
        fake.http_requests()[0].url,
        "https://api.deepl.com/v2/translate"
    );
}

#[test]
fn requests_carry_only_the_text_never_the_screenshot() {
    for (config_text, reply) in [
        (OPENAI, r#"{"choices":[{"message":{"content":"x"}}]}"#),
        (DEEPL, r#"{"translations":[{"text":"x"}]}"#),
    ] {
        let fake = fake().with_http_reply(200, reply);
        let mut panel = panel_with(&fake, config_text, Locale::ZhCn);
        translate(&mut panel, &fake);

        let request = &fake.http_requests()[0];
        // 请求体是纯 UTF-8 的 JSON，很短，不含图像数据；也没有 multipart 之类的文件部分。
        let body = std::str::from_utf8(&request.body).expect("请求体应是 UTF-8 的 JSON");
        serde_json::from_str::<serde_json::Value>(body).unwrap();
        assert!(body.len() < 600, "请求体过长：{body}");
        for forbidden in ["base64", "image", "PNG", "data:"] {
            assert!(!body.contains(forbidden), "请求体含 {forbidden}：{body}");
        }
        assert!(
            request
                .headers
                .iter()
                .all(|(_, v)| !v.contains("image") && !v.contains("multipart"))
        );
    }
}

#[test]
fn edited_text_is_what_gets_translated() {
    let fake = fake().with_http_reply(200, r#"{"translations":[{"text":"Hi"}]}"#);
    let mut panel = panel_with(&fake, DEEPL, Locale::ZhCn);
    *panel.text_mut() = "你好".to_owned();

    translate(&mut panel, &fake);

    let body = json(&fake.http_requests()[0].body);
    assert_eq!(body["text"], serde_json::json!(["你好"]));
}

#[test]
fn translate_is_unavailable_without_a_configured_service() {
    let fake = fake();
    // 没有 [translation]，或者没有填 api_key。
    for text in [
        "",
        "[translation]\nprovider = \"deepl\"\n",
        "[translation]\nprovider = \"openai\"\napi_key = \"  \"\n",
    ] {
        let mut panel = panel_with(&fake, text, Locale::ZhCn);
        assert!(!panel.can_translate(), "配置：{text:?}");
        assert!(panel.start_translation().is_none());
    }
    assert!(fake.http_requests().is_empty());
}

#[test]
fn nothing_to_translate_when_the_text_is_blank() {
    let fake = fake();
    let mut panel = panel_with(&fake, DEEPL, Locale::ZhCn);
    *panel.text_mut() = "  \n".to_owned();
    assert!(panel.start_translation().is_none());
    assert!(fake.http_requests().is_empty());
}

#[test]
fn network_failure_is_reported_and_leaves_the_text_alone() {
    let fake = fake().with_http_failure("无法连接服务器");
    let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);

    translate(&mut panel, &fake);

    let TranslationStatus::Failed(message) = panel.translation_status() else {
        panic!("应当失败：{:?}", panel.translation_status());
    };
    assert!(message.contains("无法连接服务器"), "{message}");
    assert_eq!(panel.text(), "你好，世界");
    // 失败后可以重试。
    assert!(panel.start_translation().is_some());
}

#[test]
fn service_errors_are_reported_without_leaking_the_api_key() {
    let cases = [
        (
            401,
            format!(r#"{{"error":{{"message":"Incorrect API key provided: {KEY}"}}}}"#),
        ),
        (
            429,
            format!(r#"{{"error":{{"message":"rate limit for {KEY}"}}}}"#),
        ),
        (500, "oops".to_owned()),
    ];
    for (status, body) in cases {
        let fake = fake().with_http_reply(status, &body);
        let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);
        translate(&mut panel, &fake);

        let TranslationStatus::Failed(message) = panel.translation_status() else {
            panic!("应当失败");
        };
        assert!(message.contains(&status.to_string()), "{message}");
        assert!(!message.contains(KEY), "错误提示泄漏了 API Key：{message}");
    }
}

#[test]
fn network_error_text_echoing_the_key_is_redacted() {
    let fake = fake().with_http_failure(&format!("proxy rejected {KEY}"));
    let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);
    translate(&mut panel, &fake);
    let TranslationStatus::Failed(message) = panel.translation_status() else {
        panic!("应当失败");
    };
    assert!(!message.contains(KEY), "{message}");
}

#[test]
fn unreadable_reply_is_reported() {
    let fake = fake().with_http_reply(200, "<html>not json</html>");
    let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);
    translate(&mut panel, &fake);
    assert!(matches!(
        panel.translation_status(),
        TranslationStatus::Failed(_)
    ));
}

#[test]
fn custom_base_url_and_interface_language_as_default_target() {
    let fake = fake().with_http_reply(200, r#"{"choices":[{"message":{"content":"Hi"}}]}"#);
    let mut panel = panel_with(
        &fake,
        "[translation]\nprovider = \"openai\"\napi_key = \"k\"\nbase_url = \"http://localhost:11434/v1/\"\nmodel = \"qwen\"\n",
        Locale::En,
    );

    translate(&mut panel, &fake);

    let request = &fake.http_requests()[0];
    assert_eq!(request.url, "http://localhost:11434/v1/chat/completions");
    // 目标语言缺省跟随界面语言：界面是英文时翻成英文。
    assert!(String::from_utf8_lossy(&request.body).contains("English"));
}

#[test]
fn api_key_never_appears_in_debug_output() {
    let config = Config::parse(OPENAI).unwrap();
    let fake = fake().with_http_reply(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
    let mut panel = panel_with(&fake, OPENAI, Locale::ZhCn);
    translate(&mut panel, &fake);

    assert!(!format!("{config:?}").contains(KEY));
    assert!(!format!("{:?}", fake.http_requests()).contains(KEY));
}

#[test]
fn invalid_translation_settings_report_the_line() {
    let error =
        Config::parse("[translation]\nprovider = \"google\"\napi_key = \"k\"\n").unwrap_err();
    assert_eq!(error.line(), Some(2));
    let error = Config::parse(
        "[translation]\nprovider = \"deepl\"\napi_key = \"k\"\ntarget_language = \"xx\"\n",
    )
    .unwrap_err();
    assert_eq!(error.line(), Some(4));
}
