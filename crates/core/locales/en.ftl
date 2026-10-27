# BetterPrtSc interface text: English.
# Every message here must also exist in zh-CN.ftl (a test checks that both resources match).

## Tray menu

tray-settings = Settings
tray-exit = Exit

## Notifications

notification-hotkey-failed-title = Hotkey registration failed
notification-hotkey-failed-line = { $hotkey }: { $reason }
notification-capture-failed-title = Capture failed
notification-saved-title = Screenshot saved
notification-save-failed-title = Auto-save failed
notification-recording-saved-title = Recording saved
notification-recording-in-progress-title = Recording in progress
notification-config-unwatchable-title = Cannot watch the configuration file
notification-config-unwatchable-body = Restart BetterPrtSc after editing the configuration file. { $error }
notification-config-unwritable-title = Cannot write the configuration file
notification-config-unwritable-body = Using the default configuration for now. { $error }
notification-config-invalid-title = The configuration file has errors
notification-config-unreadable-title = Cannot read the configuration file
notification-config-startup-fallback = Using the default configuration for now. { $error }
notification-config-reload-fallback = Keeping the last valid configuration. { $error }
notification-config-deleted-title = The configuration file was deleted
notification-config-deleted-body = Keeping the current configuration; restart BetterPrtSc to recreate the default one.
notification-settings-unavailable-title = Cannot open Settings
notification-result-panel-unavailable-title = Cannot open the result panel
notification-ocr-copied-title = Copied { $count } characters
notification-ocr-copied-body = The recognized text is on the clipboard.
notification-ocr-empty-title = No text recognized
notification-ocr-empty-body = The clipboard was left unchanged.
notification-ocr-no-language-pack-title = No OCR language pack is installed
notification-ocr-no-language-pack-body = The clipboard was left unchanged. Click to open Windows "Language & region" settings and add the language you want to recognize; its OCR component is installed with it.
notification-ocr-failed-title = OCR failed
notification-update-available-title = New version available: { $version }
notification-update-available-body = You have { $current }. Click to open the release page and download it yourself.
update-unrecognized-version = Cannot recognise the version “{ $version }” on GitHub Releases
notification-prtsc-taken-title = The PrtSc key is taken by the Windows screenshot tool
notification-prtsc-taken-body = Click to open Windows Settings and turn off “Use the Print screen key to open screen capture”; BetterPrtSc then takes over the PrtSc key right away, no restart needed.
notification-prtsc-available-title = The PrtSc key is available
notification-prtsc-available-body = The Windows screenshot tool no longer uses the PrtSc key; the hotkey bindings for it are active now.
notification-not-elevated-title = Could not run as administrator
notification-not-elevated-body = { $reason }. Running without administrator rights this time: hotkeys do not work while a window running as administrator is in front.
not-elevated-after-restart = Still no administrator rights after restarting as administrator
notification-autostart-unreadable-title = Cannot read the start-with-Windows registration
notification-autostart-enabled-portable-title = BetterPrtSc now starts when you sign in
notification-autostart-enabled-portable-body = In portable mode this starts the program at its current location: if you move the program folder, turn it on again (start BetterPrtSc once from the new location).
notification-autostart-enable-failed-title = Cannot turn on start with Windows
notification-autostart-disable-failed-title = Cannot turn off start with Windows

## Capture modes and post-capture actions

mode-display = Display
mode-virtual-desktop = Virtual desktop
mode-region = Region
mode-window = Window
action-copy-to-clipboard = Copy to clipboard
action-await-confirmation = Wait for confirmation
action-record = Record
action-long-screenshot = Long screenshot
action-ocr = OCR

## Why a command failed

error-platform = System call failed: { $error }
error-no-display = No display is available
error-encode = Failed to encode the screenshot: { $error }
error-too-many-same-name-files = Too many files with the same name; no free file name for { $path }
error-invalid-request = The resident instance did not understand the request “{ $request }”; make sure bps and BetterPrtSc are the same version
error-capture-in-progress = The previous capture has not finished yet
error-recording-region-too-small = The { $width } × { $height } region is too small to record (width and height must be at least { $minimum } pixels)
error-nothing-recorded = Nothing was recorded
error-recording-in-progress-hotkey = Recording in progress; press { $hotkey } to stop it before capturing
error-recording-in-progress = Recording in progress; stop it before capturing

## Why a hotkey could not be registered

registration-duplicate = Same hotkey as binding { $index }
registration-taken = Already in use by another program
registration-taken-by-system-screenshot-tool = Taken by the Windows screenshot tool (you can turn this off in Windows Settings)

## Configuration file and settings errors

config-error-at-line = Line { $line }: { $message }
config-hotkey-not-a-modifier = In hotkey “{ $hotkey }”, “{ $part }” is not a modifier (Ctrl, Alt, Shift, Win)
config-hotkey-missing-key = Hotkey “{ $hotkey }” has no main key
config-hotkey-unsupported-key = In hotkey “{ $hotkey }”, “{ $key }” is not a supported key
config-hotkey-typing-key = Hotkey “{ $hotkey }” would swallow normal typing: letter and digit keys need Ctrl, Alt or Win
config-unknown-format = Unknown output format “{ $format }”: screenshots can use png, jpg, webp, avif; recordings can use mp4, gif
config-image-format-for-recording = The output_format of a recording can only be mp4 or gif
config-legacy-actions = The old actions = [...] form is no longer supported: screenshot bindings now use after = "copy_to_clipboard" or "await_confirmation" (auto-save is auto_save = true), direct functions use function = "record", "long_screenshot" or "ocr"
config-unknown-after = Unknown post-capture action "{ $after }"; choose copy_to_clipboard or await_confirmation
config-unknown-function = Unknown direct function "{ $function }"; choose record, long_screenshot or ocr
config-after-with-function = Write either after or function, not both: after for screenshot bindings, function for direct functions
config-auto-save-not-applicable = The direct function { $function } does not support auto_save (recordings are always saved, OCR never is)
config-recording-format-for-screenshot = mp4 and gif are only for recordings (bindings with function = "record"); screenshots can use png, jpg, webp, avif
config-recording-format-in-global = The global output_format is for screenshots only (png, jpg, webp, avif); put recording formats (mp4, gif) in bindings with function = "record"
config-zero-frame-rate = frame_rate must be greater than 0
config-unknown-language = Unknown interface language “{ $language }”: use auto (follow the system), zh-CN or en
template-unclosed = A { "{" } in the filename template has no matching { "}" }
template-unmatched-closing = The filename template has a lone { "}" }; write literal braces as { "{{" } or { "}}" }
template-unknown-placeholder = Unknown placeholder { "{" }{ $name }{ "}" } in the filename template; use { "{" }mode{ "}" }, { "{" }window_title{ "}" } or a date-time format made of yyyy, yy, MM, M, dd, d, HH, H, mm, m, ss, s, fff

## Settings window: problems found before saving

settings-problem-empty-save-dir = The auto-save folder cannot be empty; choose “Default folder” if you do not want to set one
settings-problem-typing-key = Binding { $index } ({ $hotkey }): letter and digit keys need Ctrl, Alt or Win
settings-problem-missing-hotkey = Binding { $index } has no hotkey
settings-problem-duplicate-hotkey = Binding { $index } ({ $hotkey }) uses the same hotkey as binding { $other }; pick another combination
settings-group-general = General
settings-group-hotkeys = Hotkeys
settings-group-saving = Capture & saving
settings-group-about = About
settings-about-license = License: { $license }
settings-about-project = Project page
settings-hotkey-unset = Not set
settings-hotkey-record-tooltip = Click, then press the new key combination
settings-record-prompt = Press the key combination…
settings-record-help = Esc cancels, Backspace / Delete clears
settings-record-unsupported = That key is not supported: hotkeys can use PrintScreen, F1–F24, A–Z, 0–9
settings-record-unavailable = Cannot record hotkeys: { $error }
settings-problem-invalid = Cannot produce a valid configuration file: { $error }

## Settings window

settings-window-title = BetterPrtSc Settings
settings-resident-not-running = BetterPrtSc is not running: saved changes take effect the next time it starts, and only then will you know whether the hotkeys could be registered.
settings-section-general = General
settings-language = Interface language
settings-language-auto = Follow the system
settings-autostart = Start when I sign in
settings-run-as-admin = Run as administrator
settings-run-as-admin-hint = Lets the hotkeys work while a window running as administrator is in front. Asks for administrator rights once when turned on.
settings-run-as-admin-risk = Warning: programs without administrator rights can modify the folder BetterPrtSc is in, and could use that to gain administrator rights the next time you sign in. Consider reinstalling “for all users” (into Program Files) before turning this on.
settings-check-for-updates = Check for updates
settings-check-for-updates-hint = Only asks GitHub Releases for the latest version number and tells you when there is a new one; nothing is downloaded. When off, BetterPrtSc makes no network requests at all.
settings-section-ocr = OCR
settings-ocr-auto-copy = Copy recognized text automatically, without the panel
settings-ocr-auto-copy-hint = Only affects direct OCR hotkeys: the recognized text goes straight to the clipboard and a notification shows the character count. OCR and Translate on the screenshot toolbar still open the result panel.
settings-update-checking = Checking for updates…
settings-update-up-to-date = You have the latest version ({ $version })
settings-update-available = New version available: { $version }
settings-update-open-page = Open the release page
settings-update-failed = Update check failed: { $error }
settings-version = Version { $version }
settings-section-saving = Auto-save
settings-save-dir = Save folder
settings-save-dir-default = Default folder (BetterPrtSc in your Pictures folder)
settings-save-dir-custom = Custom folder
settings-browse = Browse…
settings-choose-folder = Choose the auto-save folder
settings-filename-template = File name
settings-filename-example = Example: { $name } (rendered with a fixed sample time, 2026-01-01 12:34:56, not the current time)
settings-filename-help = Placeholders: { "{" }mode{ "}" } (capture mode), { "{" }window_title{ "}" } (window title), and date-times such as { "{" }yyyy-MM-dd HH-mm-ss{ "}" } (yyyy, MM, dd, HH, mm, ss, fff and so on).
settings-output-format = Screenshot format
settings-output-format-hint = PNG and WebP are lossless; JPEG is small but lossy and has no transparency; AVIF is the smallest but slow to encode. Recording formats are set per hotkey binding.
settings-section-bindings = Hotkeys
settings-bindings-hint = Click a hotkey box, then press the key combination to record it. Letter and digit keys need Ctrl, Alt or Win.
settings-binding-failed = Not registered: { $reason }
settings-binding-add = Add hotkey
settings-binding-remove = Remove
settings-binding-hotkey = Hotkey
settings-binding-mode = Capture mode
settings-binding-actions = After capture / function
settings-binding-auto-save = Auto-save
settings-binding-needs-modifier = Letter and digit keys need Ctrl, Alt or Win
settings-config-broken = The configuration file has errors and cannot be edited here
settings-config-broken-hint = Fix the file below in a text editor, then click “Reload”.
settings-config-not-utf8 = The configuration file is not UTF-8 encoded
settings-config-unreadable = Cannot read the configuration file: { $error }
settings-changed-elsewhere = Another program changed the configuration file after Settings opened, so nothing was saved to avoid overwriting those changes. Reload to continue (this discards the changes made here).
settings-fix-problems = Fix the problems marked in red before saving.
settings-save = Save
settings-revert = Undo changes
settings-reload = Reload
settings-open-config-folder = Open configuration folder
settings-saved = Saved. The changes are in effect.
settings-saved-not-running = Saved. The changes take effect the next time BetterPrtSc starts.
settings-save-failed = Cannot save: { $error }
settings-unsaved-title = Unsaved changes
settings-unsaved-body = Save your changes before closing?
settings-save-and-close = Save and close
settings-discard-and-close = Don't save
settings-cancel = Cancel

## 选区遮罩
overlay-selected-hint = Enter or double-click the selection: done   Esc: cancel
overlay-copy-color-hint = C: copy color
overlay-color-copied = Color copied
overlay-color-copy-failed = Copy failed
overlay-record-hint = Enter or double-click the selection: start recording   Esc: cancel

## Recording toolbar
recording-toolbar-pause = Pause
recording-toolbar-resume = Resume
recording-toolbar-stop = Stop

## Annotation

annotation-rectangle = Rectangle
annotation-arrow = Arrow
annotation-mosaic = Mosaic
annotation-text = Text
annotation-select = Select
annotation-undo = Undo
annotation-redo = Redo
annotation-cancel = Cancel
annotation-save-as = Save as
annotation-confirm = Confirm
annotation-record = Record
annotation-long-screenshot = Long shot
annotation-ocr = OCR
annotation-translate = Translate
save-as-file-type = { $format } image
notification-save-as-failed-title = Could not save the screenshot

## Result panel (OCR)

result-panel-window-title = OCR result
result-panel-language = Recognition language
result-panel-recognizing = Recognizing…
result-panel-recognized = Done
result-panel-no-text = No text was recognized
result-panel-failed = Recognition failed: { $error }
result-panel-no-language-pack-title = No OCR language pack is installed
result-panel-no-language-pack-body = BetterPrtSc uses the text recognition built into Windows, which needs the OCR component of each language you want to recognize. Open Settings > Time & language > Language & region and add the language (for example “English (United States)” or “中文(简体，中国)”); Windows installs its OCR component along with it. For a language you have already added, check under its Language options that “Optical character recognition” is installed. Then click “Recognize again” — no restart needed.
result-panel-open-language-settings = Open language settings
result-panel-recognize-again = Recognize again
result-panel-copy = Copy
result-panel-copied = Copied to the clipboard
result-panel-copy-failed = Copy failed: { $error }
result-panel-auto-copy = Direct OCR: copy automatically, without the panel
result-panel-auto-copy-failed = Cannot save the setting: { $error }
result-panel-open-settings-failed = Cannot open Windows Settings: { $error }
result-panel-close = Close
config-unknown-translation-provider = Unknown translation service “{ $provider }”: use openai (OpenAI-compatible API) or deepl
config-unknown-target-language = Unknown target language “{ $language }”: use auto (follow the interface language), zh-CN, en, ja, ko, fr, de, es or ru
translation-error-network = Cannot reach the translation service: { $detail }
translation-error-rejected = The translation service rejected the API key (HTTP { $status }). Check api_key under [translation] in config.toml
translation-error-http = The translation service returned an error (HTTP { $status })
translation-error-http-detail = The translation service returned an error (HTTP { $status }): { $detail }
translation-error-reply = Cannot understand the reply from the translation service
result-panel-translate = Translate
result-panel-translate-unconfigured = No translation service is configured: fill in a service (openai or deepl) and your own API key under [translation] in config.toml
result-panel-translate-unconfigured-hint = No translation service (see [translation] in config.toml)
result-panel-translating = Translating…
result-panel-translation = Translation
result-panel-translation-failed = Translation failed: { $error }
result-panel-copy-translation = Copy translation
## Long screenshot

long-screenshot-start = Start
long-screenshot-finish = Done
long-screenshot-cancel = Cancel
long-screenshot-size = Stitched { $width } × { $height }
long-screenshot-lost = Scrolled too fast, scroll back a little
notification-long-screenshot-full-title = Long screenshot reached the height limit
notification-long-screenshot-full-body = Long screenshots are at most { $limit } pixels tall; stitching stopped automatically.

# bps command line
config-toml-error = Malformed config file: { $message }
cli-usage =
    Usage: bps capture <capture mode>

    Forwards the capture command to the running BetterPrtSc instance and behaves like pressing the matching hotkey:
    it uses the post-capture actions of the first hotkey binding for that capture mode, or copies to the clipboard if there is none.
    The resident instance is started first if it is not running.
cli-usage-modes = Capture modes:
cli-exit-codes = Exit codes:
cli-exit-success = Success
cli-exit-failed = The resident instance failed to run the command
cli-exit-usage = Invalid arguments
cli-exit-unreachable = Could not reach or start the resident instance
cli-exit-cancelled = The user cancelled the capture (for example pressed Esc on the overlay)
cli-cancelled = Cancelled
cli-failed = Failed: { $message }
cli-unreachable = Could not forward the command to the resident instance: { $error }
cli-unknown-mode = Unknown capture mode: { $mode }
cli-missing-mode = Missing capture mode
cli-extra-argument = Unexpected argument: { $argument }
cli-missing-command = Missing command
cli-unknown-command = Unknown command: { $command }
