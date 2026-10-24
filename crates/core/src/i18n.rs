//! 界面语言与界面文本。
//!
//! 文本资源用 Fluent 格式，放在 `crates/core/locales/<语言>.ftl`，编译时嵌入程序。
//! 常驻实例（托盘菜单、通知）和设置界面共用这一套资源。
//! 配置里的 [`Language`] 可以是“跟随系统”或指定语言，解析成实际使用的 [`Locale`]。

use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

const ZH_CN_FTL: &str = include_str!("../locales/zh-CN.ftl");
const EN_FTL: &str = include_str!("../locales/en.ftl");

/// 配置里的界面语言：`auto`（默认，跟随系统）、`zh-CN`、`en`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// 跟随系统的界面语言；系统语言既不是中文也不是英文时用英文。
    #[default]
    Auto,
    ZhCn,
    En,
}

impl Language {
    /// 设置界面里可选的全部取值。
    pub const ALL: [Language; 3] = [Language::Auto, Language::ZhCn, Language::En];

    /// 配置文件里的写法。
    pub fn config_name(self) -> &'static str {
        match self {
            Language::Auto => "auto",
            Language::ZhCn => "zh-CN",
            Language::En => "en",
        }
    }

    /// 实际使用的语言。`system_languages` 是系统偏好的界面语言（BCP 47，按优先级排列）。
    pub fn resolve(self, system_languages: &[String]) -> Locale {
        match self {
            Language::Auto => Locale::from_system(system_languages),
            Language::ZhCn => Locale::ZhCn,
            Language::En => Locale::En,
        }
    }
}

/// 配置文件里的界面语言无法识别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLanguage(pub String);

impl FromStr for Language {
    type Err = UnknownLanguage;

    /// 不区分大小写。
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Language::ALL
            .into_iter()
            .find(|l| l.config_name().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownLanguage(name.to_owned()))
    }
}

/// 有文本资源的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    /// 简体中文。
    ZhCn,
    /// 英文。
    En,
}

impl Locale {
    /// 按系统偏好的顺序找第一个有资源的语言：任何 `zh` 开头的都用简体中文，
    /// `en` 开头的用英文；都没有时用英文。
    pub fn from_system(system_languages: &[String]) -> Locale {
        system_languages
            .iter()
            .find_map(|tag| {
                let primary = tag.split(['-', '_']).next().unwrap_or_default();
                if primary.eq_ignore_ascii_case("zh") {
                    Some(Locale::ZhCn)
                } else if primary.eq_ignore_ascii_case("en") {
                    Some(Locale::En)
                } else {
                    None
                }
            })
            .unwrap_or(Locale::En)
    }

    fn tag(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::En => "en",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Locale::ZhCn => ZH_CN_FTL,
            Locale::En => EN_FTL,
        }
    }
}

/// 一种语言的全部界面文本。用 [`Texts::of`] 取得，首次使用时解析资源并缓存。
pub struct Texts {
    locale: Locale,
    bundle: FluentBundle<FluentResource>,
}

impl Texts {
    /// `locale` 的界面文本。
    pub fn of(locale: Locale) -> &'static Texts {
        static ZH_CN: OnceLock<Texts> = OnceLock::new();
        static EN: OnceLock<Texts> = OnceLock::new();
        let cell = match locale {
            Locale::ZhCn => &ZH_CN,
            Locale::En => &EN,
        };
        cell.get_or_init(|| Texts::load(locale))
    }

    fn load(locale: Locale) -> Texts {
        let resource =
            FluentResource::try_new(locale.source().to_owned()).unwrap_or_else(|(_, errors)| {
                panic!("{} 的文本资源有语法错误：{errors:?}", locale.tag())
            });
        let id: LanguageIdentifier = locale.tag().parse().expect("语言标识有效");
        let mut bundle = FluentBundle::new_concurrent(vec![id]);
        // 不插入 Unicode 隔离符：托盘气泡和菜单会把它们显示成方块。
        bundle.set_use_isolating(false);
        bundle
            .add_resource(resource)
            .unwrap_or_else(|errors| panic!("{} 的文本资源有重复的消息：{errors:?}", locale.tag()));
        Texts { locale, bundle }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// 没有参数的文本。
    pub fn get(&self, id: &str) -> String {
        self.format(id, &[])
    }

    /// 带参数的文本；参数按 `Display` 转换成字符串。
    pub fn format(&self, id: &str, args: &[(&str, &dyn fmt::Display)]) -> String {
        let Some(pattern) = self.bundle.get_message(id).and_then(|m| m.value()) else {
            debug_assert!(false, "{} 的文本资源里没有消息 {id}", self.locale.tag());
            return id.to_owned();
        };
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, FluentValue::from(value.to_string()));
        }
        let mut errors = Vec::new();
        let text = self
            .bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors);
        debug_assert!(errors.is_empty(), "格式化消息 {id} 出错：{errors:?}");
        text.into_owned()
    }

    /// 资源里所有消息的标识，供检查两种语言的资源是否一一对应。
    pub fn message_ids(&self) -> Vec<String> {
        let resource =
            FluentResource::try_new(self.locale.source().to_owned()).unwrap_or_else(|(r, _)| r);
        resource
            .entries()
            .filter_map(|entry| match entry {
                fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_owned()),
                _ => None,
            })
            .collect()
    }
}

impl fmt::Debug for Texts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Texts")
            .field("locale", &self.locale)
            .finish()
    }
}
