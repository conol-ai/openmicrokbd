//! Runtime UI language: a flat key -> (en, zh-Hans, zh-Hant, ja) table.
//!
//! Design: one row per string keeps all four translations visually adjacent
//! (easy review, no per-language drift), and `tr()` falls back to English —
//! a missing key can never blank the UI. Static live_design labels carry a
//! `tr_<key>` widget id and are re-texted by `App::apply_language`; dynamic
//! strings call `tr()` at their format site. Device/release log lines stay
//! English deliberately: they are diagnostics, quoted in bug reports.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    ZhHans,
    ZhHant,
    Ja,
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::ZhHans,
        2 => Lang::ZhHant,
        3 => Lang::Ja,
        _ => Lang::En,
    }
}

/// Map a BCP-47 tag from the OS to a supported language. Traditional-script
/// regions (TW/HK/MO) and explicit -Hant map to Traditional; other Chinese
/// tags to Simplified.
pub fn from_tag(tag: &str) -> Lang {
    let t = tag.to_ascii_lowercase();
    if t.starts_with("ja") {
        Lang::Ja
    } else if t.starts_with("zh") {
        if t.contains("hant") || t.contains("-tw") || t.contains("-hk") || t.contains("-mo") {
            Lang::ZhHant
        } else {
            Lang::ZhHans
        }
    } else {
        Lang::En
    }
}

pub fn detect() -> Lang {
    sys_locale::get_locale()
        .map(|tag| from_tag(&tag))
        .unwrap_or(Lang::En)
}

/// Translate a key for the current language, falling back to English (and,
/// for an unknown key, to the key itself so the mistake is visible).
pub fn tr(key: &str) -> &'static str {
    for row in STRINGS {
        if row.0 == key {
            let s = match lang() {
                Lang::En => row.1,
                Lang::ZhHans => row.2,
                Lang::ZhHant => row.3,
                Lang::Ja => row.4,
            };
            return if s.is_empty() { row.1 } else { s };
        }
    }
    debug_assert!(false, "missing i18n key: {key}");
    "?"
}

/// Keys whose live_design widget id is `tr_<key>` — re-texted wholesale by
/// apply_language. Everything else is wired by hand.
#[rustfmt::skip]
pub const ANON_KEYS: &[&str] = &[
    "profile", "device_map", "13_keys_3_controls", "select_a_control_to_edit_presses",
    "editing_offline", "connect_over_usb_c_to_sync_this", "choose_a_control",
    "select_a_key_dial_joystick_direction", "direction_gesture", "choose_what_the_rotator_does",
    "rotation_sets_both_directions", "rotation", "clockwise_and_counter_clockwise", "press",
    "push_the_rotator_down", "choose_what_the_joystick_does", "one_mode_covers_all_four_directions",
    "mode", "mouse_pointer_arrow_keys_or", "deflection_moves_the_pointer", "held_modifiers",
    "sent_with_every_arrow_press", "the_push_switch_is_its_own_key", "keycap",
    "choose_the_artwork_and_short", "label", "behavior", "choose_what_happens_when_you",
    "application", "shortcut", "control", "modifiers", "key", "application_2", "device_output",
    "stored_on_the_pad_and_works", "modifiers_2", "keycode_2", "media_code_2", "desktop_action",
    "optional_automation_run_by_the", "when_pressed", "needs_the_accessibility_permission",
    "label_icon", "keep_the_hardware_map_easy_to", "label_2", "joystick_sensitivity",
    "applies_to_every_direction_in", "lower_values_respond_sooner", "saved_locally", "settings_2",
    "app_behavior_profile_data_and", "keep_pad_actions_available_after",
    "switch_profiles_without_opening", "device", "dims_the_per_key_backlight_and", "profile_data",
    "accessibility", "your_human_readable_json_config", "build_a_short_dependable_action",
    "steps_run_in_order_delays_are", "firmware", "safely_update_or_recover_your", "installed",
    "profiles_and_key_configs_survive", "drops_the_pad_into_its_rom_bootloader",
    "keep_the_pad_powered_if_the", "choose_an_application_2", "backlight_pattern",
    "ambient_pattern",
];

#[rustfmt::skip]
pub const STRINGS: &[(&str, &str, &str, &str, &str)] = &[
    // ---- static chrome (anon live_design labels, id = tr_<key>) ----
    ("profile", "PROFILE", "配置", "設定檔", "プロファイル"),
    ("device_map", "Device map", "设备视图", "裝置視圖", "デバイスマップ"),
    ("13_keys_3_controls", "13 keys · 3 controls", "13 键 · 3 个控件", "13 鍵 · 3 個控制項", "13キー · 3コントロール"),
    ("select_a_control_to_edit_presses", "Select a control to edit · presses light up live", "点击控件进行编辑 · 按键实时亮起", "點選控制項進行編輯 · 按鍵即時亮起", "コントロールを選択して編集 · 押すとライブで光ります"),
    ("editing_offline", "Editing offline", "离线编辑中", "離線編輯中", "オフライン編集中"),
    ("connect_over_usb_c_to_sync_this", "Connect over USB-C to sync this profile.", "通过 USB-C 连接以同步此配置。", "透過 USB-C 連接以同步此設定檔。", "USB-C で接続するとこのプロファイルを同期します。"),
    ("choose_a_control", "Choose a control", "选择一个控件", "選擇一個控制項", "コントロールを選択"),
    ("select_a_key_dial_joystick_direction", "Select a key, dial, joystick direction, or touch input from the hardware map.", "从设备视图中选择按键、旋钮、摇杆方向或触摸输入。", "從裝置視圖中選擇按鍵、旋鈕、搖桿方向或觸控輸入。", "デバイスマップからキー、ダイヤル、ジョイスティック方向、タッチ入力を選択してください。"),
    ("direction_gesture", "DIRECTION / GESTURE", "方向 / 手势", "方向 / 手勢", "方向 / ジェスチャー"),
    ("choose_what_the_rotator_does", "Choose what the rotator does", "选择旋钮的功能", "選擇旋鈕的功能", "ロータリーの機能を選択"),
    ("rotation_sets_both_directions", "Rotation sets both directions; Press sets the push switch.", "「旋转」同时设置两个方向；「按下」设置按压开关。", "「旋轉」同時設定兩個方向；「按下」設定按壓開關。", "「回転」は両方向をまとめて設定し、「押し込み」はプッシュスイッチを設定します。"),
    ("rotation", "Rotation", "旋转", "旋轉", "回転"),
    ("clockwise_and_counter_clockwise", "Clockwise and counter-clockwise", "顺时针与逆时针", "順時針與逆時針", "時計回りと反時計回り"),
    ("press", "Press", "按下", "按下", "押し込み"),
    ("push_the_rotator_down", "Push the rotator down", "向下按压旋钮", "向下按壓旋鈕", "ロータリーを押し込む"),
    ("choose_what_the_joystick_does", "Choose what the joystick does", "选择摇杆的功能", "選擇搖桿的功能", "ジョイスティックの機能を選択"),
    ("one_mode_covers_all_four_directions", "One mode covers all four directions and the push switch.", "一个模式涵盖四个方向和按压开关。", "一個模式涵蓋四個方向和按壓開關。", "1つのモードが4方向とプッシュスイッチをカバーします。"),
    ("mode", "Mode", "模式", "模式", "モード"),
    ("mouse_pointer_arrow_keys_or", "Mouse pointer, arrow keys, or custom keys", "鼠标指针、方向键或自定义按键", "滑鼠指標、方向鍵或自訂按鍵", "マウスポインター、矢印キー、カスタムキー"),
    ("deflection_moves_the_pointer", "Deflection moves the pointer proportionally — further is faster. Pushing the stick clicks the left mouse button.", "偏移越大，指针移动越快。按下摇杆为鼠标左键单击。", "偏移越大，指標移動越快。按下搖桿為滑鼠左鍵點擊。", "倒すほどポインターが速く動きます。スティックを押し込むと左クリックになります。"),
    ("held_modifiers", "Held modifiers", "按住的修饰键", "按住的修飾鍵", "同時押しの修飾キー"),
    ("sent_with_every_arrow_press", "Sent with every arrow press — optional", "随每次方向键一同发送——可选", "隨每次方向鍵一同送出——可選", "矢印キーと同時に送信されます（任意）"),
    ("the_push_switch_is_its_own_key", "The push switch is its own key — configure it below like any other.", "按压开关是独立按键——在下方像其他按键一样配置。", "按壓開關是獨立按鍵——在下方像其他按鍵一樣設定。", "プッシュスイッチは独立したキーです。下で他のキーと同様に設定できます。"),
    ("keycap", "Keycap", "键帽", "鍵帽", "キーキャップ"),
    ("choose_the_artwork_and_short", "Choose the artwork and short label shown on the map", "选择在设备视图上显示的图标和短标签", "選擇在裝置視圖上顯示的圖示和短標籤", "マップに表示されるアイコンと短いラベルを選択"),
    ("label", "LABEL", "标签", "標籤", "ラベル"),
    ("behavior", "Behavior", "行为", "行為", "動作"),
    ("choose_what_happens_when_you", "Choose what happens when you press this control", "选择按下此控件时执行的操作", "選擇按下此控制項時執行的操作", "このコントロールを押したときの動作を選択"),
    ("application", "APPLICATION", "应用程序", "應用程式", "アプリケーション"),
    ("shortcut", "SHORTCUT", "快捷键", "快速鍵", "ショートカット"),
    ("control", "CONTROL", "控制", "控制", "コントロール"),
    ("modifiers", "MODIFIERS", "修饰键", "修飾鍵", "修飾キー"),
    ("key", "KEY", "按键", "按鍵", "キー"),
    ("application_2", "APPLICATION", "应用程序", "應用程式", "アプリケーション"),
    ("device_output", "Device output", "设备输出", "裝置輸出", "デバイス出力"),
    ("stored_on_the_pad_and_works", "Stored on the pad and works without this app", "存储在键盘上，无需本应用也能工作", "儲存在鍵盤上，無需本應用程式也能運作", "パッド本体に保存され、このアプリなしでも動作します"),
    ("modifiers_2", "MODIFIERS", "修饰键", "修飾鍵", "修飾キー"),
    ("keycode_2", "KEYCODE", "键码", "鍵碼", "キーコード"),
    ("media_code_2", "MEDIA CODE", "媒体码", "媒體碼", "メディアコード"),
    ("desktop_action", "Desktop action", "桌面动作", "桌面動作", "デスクトップアクション"),
    ("optional_automation_run_by_the", "Optional automation run by the host app", "由主机应用执行的可选自动化", "由主機應用程式執行的可選自動化", "ホストアプリが実行するオプションの自動化"),
    ("when_pressed", "WHEN PRESSED", "按下时", "按下時", "押したとき"),
    ("needs_the_accessibility_permission", "Needs the Accessibility permission — grant it from Settings (gear, below).", "需要「辅助功能」权限——请在下方设置（齿轮）中授予。", "需要「輔助使用」權限——請在下方設定（齒輪）中授予。", "アクセシビリティ権限が必要です。下の設定（歯車）から許可してください。"),
    ("label_icon", "Label & icon", "标签与图标", "標籤與圖示", "ラベルとアイコン"),
    ("keep_the_hardware_map_easy_to", "Keep the hardware map easy to scan", "让设备视图一目了然", "讓裝置視圖一目瞭然", "デバイスマップを見やすく保ちます"),
    ("label_2", "LABEL", "标签", "標籤", "ラベル"),
    ("joystick_sensitivity", "Joystick sensitivity", "摇杆灵敏度", "搖桿靈敏度", "ジョイスティック感度"),
    ("applies_to_every_direction_in", "Applies to every direction in this profile", "应用于此配置中的所有方向", "套用於此設定檔中的所有方向", "このプロファイルの全方向に適用されます"),
    ("lower_values_respond_sooner", "Lower values respond sooner. Changes are debounced and written safely to the pad.", "数值越低响应越快。更改会防抖后安全写入键盘。", "數值越低回應越快。變更會防抖後安全寫入鍵盤。", "値が小さいほど早く反応します。変更は間引かれて安全に本体へ書き込まれます。"),
    ("saved_locally", "Saved locally", "已保存到本地", "已儲存到本機", "ローカルに保存済み"),
    ("settings_2", "Settings", "设置", "設定", "設定"),
    ("app_behavior_profile_data_and", "App behavior, profile data, and permissions", "应用行为、配置数据与权限", "應用程式行為、設定檔資料與權限", "アプリの動作、プロファイルデータ、権限"),
    ("keep_pad_actions_available_after", "Keep pad actions available after sign-in.", "登录后即可使用键盘动作。", "登入後即可使用鍵盤動作。", "サインイン後すぐにパッドの操作を使えるようにします。"),
    ("switch_profiles_without_opening", "Switch profiles without opening the window.", "无需打开窗口即可切换配置。", "無需開啟視窗即可切換設定檔。", "ウィンドウを開かずにプロファイルを切り替えられます。"),
    ("device", "DEVICE", "设备", "裝置", "デバイス"),
    ("dims_the_per_key_backlight_and", "Dims the per-key backlight and the underglow ring together, live while you drag. 100% is the USB power budget cap; 0% turns the lights off. Saved on the pad.", "同时调暗按键背光和底部灯环，拖动时实时生效。100% 为 USB 功率上限；0% 关闭灯光。设置保存在键盘上。", "同時調暗按鍵背光和底部燈環，拖曳時即時生效。100% 為 USB 功率上限；0% 關閉燈光。設定儲存在鍵盤上。", "キーバックライトとアンダーグローを同時に調光し、ドラッグ中にライブ反映されます。100% は USB 電力上限、0% で消灯。パッド本体に保存されます。"),
    ("profile_data", "PROFILE DATA", "配置数据", "設定檔資料", "プロファイルデータ"),
    ("accessibility", "ACCESSIBILITY", "辅助功能", "輔助使用", "アクセシビリティ"),
    ("your_human_readable_json_config", "Your human-readable JSON config stays in the user config directory. Everything works offline.", "人类可读的 JSON 配置保存在用户配置目录中。一切均可离线使用。", "人類可讀的 JSON 設定保存在使用者設定目錄中。一切均可離線使用。", "設定は読みやすい JSON としてユーザー設定ディレクトリに保存され、すべてオフラインで動作します。"),
    ("build_a_short_dependable_action", "Build a short, dependable action sequence", "构建简短可靠的动作序列", "建立簡短可靠的動作序列", "短く確実なアクションシーケンスを作成"),
    ("steps_run_in_order_delays_are", "Steps run in order. Delays are milliseconds; Record captures a shortcut for a keystroke step.", "步骤按顺序执行。延迟单位为毫秒；「录制」可为按键步骤捕获快捷键。", "步驟按順序執行。延遲單位為毫秒；「錄製」可為按鍵步驟擷取快速鍵。", "ステップは順番に実行されます。遅延はミリ秒単位。「記録」でキー入力ステップのショートカットを取り込めます。"),
    ("firmware", "Firmware", "固件", "韌體", "ファームウェア"),
    ("safely_update_or_recover_your", "Safely update or recover your OpenMicro", "安全地更新或恢复您的 OpenMicro", "安全地更新或復原您的 OpenMicro", "OpenMicro を安全に更新・復旧します"),
    ("installed", "INSTALLED", "已安装", "已安裝", "インストール済み"),
    ("profiles_and_key_configs_survive", "Profiles and key configs survive updates — the keymap lives in a flash page the update never touches.", "配置和按键设置在更新后保留——键位图存储在更新不会触及的闪存页中。", "設定檔和按鍵設定在更新後保留——鍵位圖儲存在更新不會觸及的快閃記憶體頁中。", "プロファイルとキー設定は更新後も保持されます。キーマップは更新が触れないフラッシュページに保存されています。"),
    ("drops_the_pad_into_its_rom_bootloader", "Drops the pad into its ROM bootloader (0483:df11) and leaves it there.", "让键盘进入 ROM 引导程序（0483:df11）并停留在该模式。", "讓鍵盤進入 ROM 開機載入程式（0483:df11）並停留在該模式。", "パッドを ROM ブートローダー（0483:df11）へ移行させ、そのまま待機させます。"),
    ("keep_the_pad_powered_if_the", "Keep the pad powered. If the app stops while ROM DFU is still present, Install can resume; unplugging mid-flash may require SWD recovery.", "请保持键盘供电。若应用在 ROM DFU 模式下停止，「安装」可以续传；写入中途拔出可能需要 SWD 恢复。", "請保持鍵盤供電。若應用程式在 ROM DFU 模式下停止，「安裝」可以續傳；寫入中途拔出可能需要 SWD 復原。", "パッドの電源を切らないでください。ROM DFU 中にアプリが停止しても「インストール」で再開できます。書き込み中に抜くと SWD による復旧が必要になる場合があります。"),
    ("choose_an_application_2", "Choose an application", "选择应用程序", "選擇應用程式", "アプリケーションを選択"),
    // ---- named static widgets (wired by hand in apply_language) ----
    ("settings", "Settings", "设置", "設定", "設定"),
    ("device_deck", "DEVICE DECK", "设备面板", "裝置面板", "デバイスデッキ"),
    ("selected_control", "SELECTED CONTROL", "当前控件", "目前控制項", "選択中のコントロール"),
    ("behavior_family", "BEHAVIOR TYPE", "行为类型", "行為類型", "動作タイプ"),
    ("selected", "SELECTED", "已选择", "已選擇", "選択済み"),
    ("updating_device", "UPDATING DEVICE", "正在更新设备", "正在更新裝置", "デバイスを更新中"),
    ("downloading", "DOWNLOADING", "正在下载", "正在下載", "ダウンロード中"),
    ("installing", "INSTALLING…", "正在安装…", "正在安裝…", "インストール中…"),
    ("update_check_unavailable", "UPDATE CHECK UNAVAILABLE", "暂时无法检查更新", "暫時無法檢查更新", "更新を確認できません"),
    ("update_now", "Update now", "立即更新", "立即更新", "今すぐ更新"),
    ("later", "Later", "稍后", "稍後", "後で"),
    ("perm_banner", "Accessibility permission is needed for this behavior.", "此行为需要「辅助功能」权限。", "此行為需要「輔助使用」權限。", "この動作にはアクセシビリティ権限が必要です。"),
    ("open_settings", "Open Settings", "打开设置", "開啟設定", "設定を開く"),
    ("dial_rotator", "ROTATOR", "旋钮", "旋鈕", "ロータリー"),
    ("dial_joystick", "JOYSTICK", "摇杆", "搖桿", "ジョイスティック"),
    ("dial_touch", "TOUCH", "触摸", "觸控", "タッチ"),
    ("profile_name_placeholder", "Profile name", "配置名称", "設定檔名稱", "プロファイル名"),
    ("keycap_label_placeholder", "Short keycap label", "键帽短标签", "鍵帽短標籤", "キーキャップの短いラベル"),
    ("change_icon", "Change icon…", "更换图标…", "更換圖示…", "アイコンを変更…"),
    ("existing_setup_note", "This control keeps its existing setup until you choose a behavior.", "在您选择行为之前，此控件保持现有设置。", "在您選擇行為之前，此控制項保持現有設定。", "動作を選ぶまで、このコントロールは現在の設定を保持します。"),
    ("choose_application", "Choose an application…", "选择应用程序…", "選擇應用程式…", "アプリケーションを選択…"),
    ("refresh", "Refresh", "刷新", "重新整理", "更新"),
    ("open", "Open", "打开", "開啟", "開く"),
    ("kind_nothing", "Nothing", "无", "無", "なし"),
    ("kind_keycode", "Keycode", "键码", "鍵碼", "キーコード"),
    ("kind_media", "Media code", "媒体码", "媒體碼", "メディアコード"),
    ("record_shortcut", "Record shortcut", "录制快捷键", "錄製快速鍵", "ショートカットを記録"),
    ("test", "Test", "测试", "測試", "テスト"),
    ("edit_steps", "Edit steps…", "编辑步骤…", "編輯步驟…", "ステップを編集…"),
    ("shell_command_placeholder", "shell command", "shell 命令", "shell 命令", "シェルコマンド"),
    ("open_placeholder", "URL, file, or application", "URL、文件或应用程序", "URL、檔案或應用程式", "URL、ファイル、またはアプリ"),
    ("browse", "Browse…", "浏览…", "瀏覽…", "参照…"),
    ("pointer_speed", "Pointer speed", "指针速度", "指標速度", "ポインター速度"),
    ("deflection", "Deflection", "触发偏移", "觸發偏移", "しきい値"),
    ("backlight_brightness", "Backlight brightness", "背光亮度", "背光亮度", "バックライトの明るさ"),
    ("done", "Done", "完成", "完成", "完了"),
    ("launch_at_login", "Launch at login", "登录时启动", "登入時啟動", "ログイン時に起動"),
    ("show_menubar_icon", "Show menu bar icon", "显示菜单栏图标", "顯示選單列圖示", "メニューバーにアイコンを表示"),
    ("language", "Language", "语言", "語言", "言語"),
    ("language_auto", "Auto (system)", "自动（跟随系统）", "自動（跟隨系統）", "自動（システムに合わせる）"),
    ("export", "Export…", "导出…", "匯出…", "書き出す…"),
    ("import_replace", "Import (replace)…", "导入（替换）…", "匯入（取代）…", "読み込む（置き換え）…"),
    ("import_merge", "Import (merge)…", "导入（合并）…", "匯入（合併）…", "読み込む（統合）…"),
    ("reset_factory", "Reset all bindings to factory defaults", "将所有绑定重置为出厂默认", "將所有綁定重設為出廠預設", "すべての割り当てを工場出荷時に戻す"),
    ("reset_confirm", "Really reset everything?", "确定要全部重置吗？", "確定要全部重設嗎？", "本当にすべてリセットしますか？"),
    ("open_system_settings", "Open System Settings", "打开系统设置", "開啟系統設定", "システム設定を開く"),
    ("cancel", "Cancel", "取消", "取消", "キャンセル"),
    ("add_step", "Add step", "添加步骤", "新增步驟", "ステップを追加"),
    ("test_run", "Test run", "试运行", "試執行", "テスト実行"),
    ("close", "Close", "关闭", "關閉", "閉じる"),
    ("choose_bin", "Choose .bin…", "选择 .bin…", "選擇 .bin…", ".bin を選択…"),
    ("bin_hint", "a raw .bin built from the fw crate", "由 fw crate 构建的原始 .bin", "由 fw crate 建置的原始 .bin", "fw クレートからビルドした raw .bin"),
    ("no_image_selected", "No image selected", "未选择镜像", "未選擇映像", "イメージ未選択"),
    ("install", "Install", "安装", "安裝", "インストール"),
    ("advanced", "Advanced", "高级", "進階", "詳細"),
    ("reboot_into_dfu", "Reboot into DFU", "重启进入 DFU", "重新啟動進入 DFU", "DFU モードで再起動"),
    ("app_picker_meta", "Search or scroll through installed apps", "搜索或滚动浏览已安装的应用", "搜尋或捲動瀏覽已安裝的應用程式", "インストール済みアプリを検索またはスクロール"),
    ("app_search_placeholder", "Search installed apps", "搜索已安装的应用", "搜尋已安裝的應用程式", "インストール済みアプリを検索"),
    ("clear", "Clear", "清除", "清除", "クリア"),
    ("icon", "Icon", "图标", "圖示", "アイコン"),
    ("no_icon", "No icon", "无图标", "無圖示", "アイコンなし"),
    ("no_icon_lower", "no icon", "无图标", "無圖示", "アイコンなし"),
    ("icon_search_placeholder", "Search icons — e.g. mic, git, arrow", "搜索图标——例如 mic、git、arrow", "搜尋圖示——例如 mic、git、arrow", "アイコンを検索 — 例: mic、git、arrow"),
    ("brand_search_placeholder", "Search brand logos — e.g. GitHub, Apple, Figma", "搜索品牌标志——例如 GitHub、Apple、Figma", "搜尋品牌標誌——例如 GitHub、Apple、Figma", "ブランドロゴを検索 — 例: GitHub、Apple、Figma"),
    ("lucide_icons", "Lucide", "Lucide", "Lucide", "Lucide"),
    ("simple_icons", "Simple Icons", "Simple Icons", "Simple Icons", "Simple Icons"),
    ("no_brand_icons", "NO BRAND LOGOS", "没有匹配的品牌标志", "沒有符合的品牌標誌", "一致するブランドロゴなし"),
    ("brand_search_hint", "Try a company, product, or technology name.", "尝试搜索公司、产品或技术名称。", "嘗試搜尋公司、產品或技術名稱。", "会社名、製品名、技術名をお試しください。"),
    ("enabled", "Enabled", "启用", "啟用", "有効"),
    ("record", "Record", "录制", "錄製", "記録"),
    // ---- dynamic strings ----
    ("searching", "Searching…", "搜索中…", "搜尋中…", "検索中…"),
    ("waiting_for_device", "Waiting for device", "等待设备", "等待裝置", "デバイスを待機中"),
    ("live", "Live", "已连接", "已連接", "接続中"),
    ("connected", "Connected", "已连接", "已連接", "接続済み"),
    ("rotator_hdr", "Rotator", "旋钮", "旋鈕", "ロータリー"),
    ("rotation_plus_press", "Rotation + press", "旋转 + 按下", "旋轉 + 按下", "回転 + 押し込み"),
    ("joystick_hdr", "Joystick", "摇杆", "搖桿", "ジョイスティック"),
    ("mouse_pointer_mode", "Mouse pointer", "鼠标指针", "滑鼠指標", "マウスポインター"),
    ("unlabelled", "Unlabelled", "未命名", "未命名", "ラベルなし"),
    ("custom_existing", "Custom (existing)", "自定义（现有）", "自訂（現有）", "カスタム（現在の設定）"),
    ("st_pass_through", "No host action", "无主机动作", "無主機動作", "ホスト動作なし"),
    ("st_active", "Host action active", "主机动作已启用", "主機動作已啟用", "ホスト動作が有効"),
    ("st_consumer", "Handled by OS", "由系统处理", "由系統處理", "OS が処理"),
    ("st_dead", "Invisible on this OS", "此系统不可见", "此系統不可見", "この OS では検出不可"),
    ("st_nothing", "Emits nothing", "无输出", "無輸出", "何も送信しない"),
    ("st_taken", "Key already taken", "按键已被占用", "按鍵已被佔用", "キーは使用中"),
    ("st_unavailable", "Hotkeys unavailable", "热键不可用", "熱鍵不可用", "ホットキー利用不可"),
    ("mode_mouse", "Mouse pointer", "鼠标指针", "滑鼠指標", "マウスポインター"),
    ("mode_arrows", "Arrow keys", "方向键", "方向鍵", "矢印キー"),
    ("mode_custom", "Custom keys", "自定义按键", "自訂按鍵", "カスタムキー"),
    ("joy_mouse_detail", "Deflection moves the mouse pointer; pushing the stick is a left click. Works on any machine, app running or not.", "偏移摇杆移动鼠标指针；按下摇杆为左键单击。在任何电脑上均可使用，无需运行本应用。", "偏移搖桿移動滑鼠指標；按下搖桿為左鍵點擊。在任何電腦上均可使用，無需執行本應用程式。", "スティックを倒すとポインターが動き、押し込むと左クリック。アプリなしでもどのマシンでも動作します。"),
    ("joy_arrows_detail", "Deflection sends the arrow keys, with any held modifiers you choose; pushing the stick is its own configurable key.", "偏移摇杆发送方向键，可附加所选修饰键；按下摇杆是独立可配置按键。", "偏移搖桿送出方向鍵，可附加所選修飾鍵；按下搖桿是獨立可設定按鍵。", "スティックを倒すと矢印キーを送信（修飾キー付加可）。押し込みは独立した設定可能キーです。"),
    ("joy_custom_detail", "Each direction and the push switch is its own fully configurable key — pick one with the direction selector below.", "每个方向和按压开关都是完全可配置的独立按键——用下方的方向选择器进行选择。", "每個方向和按壓開關都是完全可設定的獨立按鍵——用下方的方向選擇器進行選擇。", "各方向とプッシュスイッチは個別に設定可能なキーです。下の方向セレクターで選択してください。"),
    ("sync_connected_note", "Changes are written to OpenMicro and saved in its flash memory.", "更改会写入 OpenMicro 并保存到其闪存中。", "變更會寫入 OpenMicro 並儲存到其快閃記憶體中。", "変更は OpenMicro に書き込まれ、フラッシュメモリに保存されます。"),
    ("sync_offline_note", "Saved in this profile · connect OpenMicro to write these choices to the keyboard.", "已保存到此配置 · 连接 OpenMicro 后写入键盘。", "已儲存到此設定檔 · 連接 OpenMicro 後寫入鍵盤。", "このプロファイルに保存済み · OpenMicro を接続すると本体へ書き込まれます。"),
    ("preset_volume", "Volume", "音量", "音量", "音量"),
    ("preset_brightness", "Screen brightness", "屏幕亮度", "螢幕亮度", "画面の明るさ"),
    ("preset_tracks", "Track selection", "曲目切换", "曲目切換", "トラック選択"),
    ("preset_v_arrows", "Arrow keys · up / down", "方向键 · 上 / 下", "方向鍵 · 上 / 下", "矢印キー · 上 / 下"),
    ("preset_h_arrows", "Arrow keys · left / right", "方向键 · 左 / 右", "方向鍵 · 左 / 右", "矢印キー · 左 / 右"),
    ("press_mute", "Mute", "静音", "靜音", "ミュート"),
    ("press_lock", "Lock screen", "锁定屏幕", "鎖定螢幕", "画面をロック"),
    ("press_play", "Play", "播放", "播放", "再生"),
    ("press_enter", "Enter", "回车", "Enter", "Enter"),
    ("act_none", "Do nothing", "无操作", "無操作", "何もしない"),
    ("act_keystroke", "Keystroke", "按键", "按鍵", "キー入力"),
    ("act_macro", "Macro", "宏", "巨集", "マクロ"),
    ("act_run", "Run command", "运行命令", "執行命令", "コマンドを実行"),
    ("act_open", "Open app or URL", "打开应用或 URL", "開啟應用程式或 URL", "アプリ / URL を開く"),
    ("act_media", "Media control", "媒体控制", "媒體控制", "メディア操作"),
    ("act_app_settings", "App settings", "应用设置", "應用程式設定", "アプリ設定"),
    ("slot_key_n", "Key {n} · row {r}", "按键 {n} · 第 {r} 行", "按鍵 {n} · 第 {r} 列", "キー {n} · 行 {r}"),
    ("slot_enc_cw", "Encoder · clockwise", "旋钮 · 顺时针", "旋鈕 · 順時針", "ロータリー · 時計回り"),
    ("slot_enc_ccw", "Encoder · counter-clockwise", "旋钮 · 逆时针", "旋鈕 · 逆時針", "ロータリー · 反時計回り"),
    ("slot_enc_press", "Encoder · press", "旋钮 · 按下", "旋鈕 · 按下", "ロータリー · 押し込み"),
    ("slot_joy_up", "Joystick · up", "摇杆 · 上", "搖桿 · 上", "ジョイスティック · 上"),
    ("slot_joy_down", "Joystick · down", "摇杆 · 下", "搖桿 · 下", "ジョイスティック · 下"),
    ("slot_joy_left", "Joystick · left", "摇杆 · 左", "搖桿 · 左", "ジョイスティック · 左"),
    ("slot_joy_right", "Joystick · right", "摇杆 · 右", "搖桿 · 右", "ジョイスティック · 右"),
    ("slot_joy_press", "Joystick · press", "摇杆 · 按下", "搖桿 · 按下", "ジョイスティック · 押し込み"),
    ("slot_touch_tap", "Touch pad · tap", "触摸板 · 轻点", "觸控板 · 輕點", "タッチパッド · タップ"),
    ("slot_touch_swipe_l", "Touch pad · swipe left", "触摸板 · 左滑", "觸控板 · 左滑", "タッチパッド · 左スワイプ"),
    ("slot_touch_swipe_r", "Touch pad · swipe right", "触摸板 · 右滑", "觸控板 · 右滑", "タッチパッド · 右スワイプ"),
    ("perm_granted", "Accessibility / Input Monitoring: granted — keystroke and media actions can run.", "辅助功能 / 输入监控：已授权——按键和媒体动作可以执行。", "輔助使用 / 輸入監控：已授權——按鍵和媒體動作可以執行。", "アクセシビリティ / 入力監視：許可済み。キー入力とメディア操作を実行できます。"),
    ("perm_missing", "Accessibility / Input Monitoring: not granted — the app can show state but cannot type or press media keys for you.", "辅助功能 / 输入监控：未授权——应用可显示状态，但无法为您输入按键或媒体键。", "輔助使用 / 輸入監控：未授權——應用程式可顯示狀態，但無法為您輸入按鍵或媒體鍵。", "アクセシビリティ / 入力監視：未許可。状態表示は可能ですが、キー入力やメディアキーの送信はできません。"),
    ("n_steps", "{n} steps", "{n} 个步骤", "{n} 個步驟", "{n} ステップ"),
    ("one_step", "1 step", "1 个步骤", "1 個步驟", "1 ステップ"),
    ("press_keys", "press keys…", "请按下按键…", "請按下按鍵…", "キーを押してください…"),
    ("backlight_pattern", "Backlight pattern", "背光模式", "背光模式", "バックライトパターン"),
    ("ambient_pattern", "Ambient light", "氛围灯", "氣氛燈", "アンビエントライト"),
    ("pattern_key_note", "Idle pattern of the key backlight · presses still pop white", "按键背光的空闲模式 · 按下仍会亮白色", "按鍵背光的閒置模式 · 按下仍會亮白色", "キーバックライトの待機パターン · 押すと白く光ります"),
    ("pattern_ambient_note", "The underglow ring around the board", "键盘四周的底部灯环", "鍵盤四周的底部燈環", "ボード周囲のアンダーグローリング"),
    ("pat_rainbow", "Rainbow", "彩虹", "彩虹", "レインボー"),
    ("pat_white", "White", "白色", "白色", "ホワイト"),
    ("pat_red", "Red", "红色", "紅色", "レッド"),
    ("pat_orange", "Orange", "橙色", "橙色", "オレンジ"),
    ("pat_yellow", "Yellow", "黄色", "黃色", "イエロー"),
    ("pat_green", "Green", "绿色", "綠色", "グリーン"),
    ("pat_cyan", "Cyan", "青色", "青色", "シアン"),
    ("pat_blue", "Blue", "蓝色", "藍色", "ブルー"),
    ("pat_purple", "Purple", "紫色", "紫色", "パープル"),
    ("pat_pink", "Pink", "粉色", "粉色", "ピンク"),
    ("mb_open", "Open OpenMicro", "打开 OpenMicro", "開啟 OpenMicro", "OpenMicro を開く"),
    ("mb_quit", "Quit OpenMicro", "退出 OpenMicro", "結束 OpenMicro", "OpenMicro を終了"),
    ("choose_icon", "Choose icon…", "选择图标…", "選擇圖示…", "アイコンを選択…"),
    ("short_label", "Short label", "短标签", "短標籤", "短いラベル"),
    ("language_eyebrow", "LANGUAGE", "语言", "語言", "言語"),
    ("language_applies", "Applies immediately · Auto follows the system", "立即生效 · 「自动」跟随系统语言", "立即生效 · 「自動」跟隨系統語言", "すぐに反映 · 「自動」はシステム言語に従います"),
    ("appearance_eyebrow", "APPEARANCE", "外观", "外觀", "外観"),
    ("theme_applies", "Applies immediately · System follows the OS appearance", "立即生效 · 「跟随系统」会匹配操作系统外观", "立即生效 · 「跟隨系統」會配合作業系統外觀", "すぐに反映 · 「システム」は OS の外観に従います"),
    ("theme_system", "System", "跟随系统", "跟隨系統", "システム"),
    ("theme_light", "Light", "浅色", "淺色", "ライト"),
    ("theme_dark", "Dark", "深色", "深色", "ダーク"),
    ("rotd_volume", "Clockwise raises volume · counter-clockwise lowers it.", "顺时针调高音量 · 逆时针调低。", "順時針調高音量 · 逆時針調低。", "時計回りで音量アップ · 反時計回りでダウン。"),
    ("rotd_brightness", "Clockwise brightens the screen · counter-clockwise dims it.", "顺时针调亮屏幕 · 逆时针调暗。", "順時針調亮螢幕 · 逆時針調暗。", "時計回りで画面が明るく · 反時計回りで暗くなります。"),
    ("rotd_tracks", "Clockwise selects the next song · counter-clockwise selects the previous song.", "顺时针切换到下一曲 · 逆时针切换到上一曲。", "順時針切換到下一曲 · 逆時針切換到上一曲。", "時計回りで次の曲 · 反時計回りで前の曲。"),
    ("rotd_v_arrows", "Clockwise sends Arrow Down · counter-clockwise sends Arrow Up.", "顺时针发送下方向键 · 逆时针发送上方向键。", "順時針送出下方向鍵 · 逆時針送出上方向鍵。", "時計回りで下矢印 · 反時計回りで上矢印を送信。"),
    ("rotd_h_arrows", "Clockwise sends Arrow Right · counter-clockwise sends Arrow Left.", "顺时针发送右方向键 · 逆时针发送左方向键。", "順時針送出右方向鍵 · 逆時針送出左方向鍵。", "時計回りで右矢印 · 反時計回りで左矢印を送信。"),
    ("rot_custom_detail", "The existing clockwise and counter-clockwise mappings are preserved.", "保留现有的顺时针与逆时针映射。", "保留現有的順時針與逆時針對應。", "既存の時計回り / 反時計回りの割り当てを保持します。"),
    ("pressd_mute", "Sends the system audio Mute command.", "发送系统静音命令。", "送出系統靜音命令。", "システムのミュートコマンドを送信します。"),
    ("pressd_lock", "Sends the standard HID lock-screen command. Support depends on the operating system.", "发送标准 HID 锁屏命令。支持情况取决于操作系统。", "送出標準 HID 鎖定螢幕命令。支援情況取決於作業系統。", "標準 HID の画面ロックコマンドを送信します。対応は OS によります。"),
    ("pressd_play", "Sends the dedicated media Play command.", "发送专用的媒体播放命令。", "送出專用的媒體播放命令。", "専用のメディア再生コマンドを送信します。"),
    ("pressd_enter", "Sends the standard Enter key.", "发送标准回车键。", "送出標準 Enter 鍵。", "標準の Enter キーを送信します。"),
    ("press_custom_detail", "The existing push-switch mapping is preserved.", "保留现有的按压开关映射。", "保留現有的按壓開關對應。", "既存のプッシュスイッチの割り当てを保持します。"),
    ("rot_custom_both", "This profile has custom rotation and press mappings. Both stay untouched until you choose a preset.", "此配置有自定义的旋转和按下映射。在您选择预设前均保持不变。", "此設定檔有自訂的旋轉和按下對應。在您選擇預設前均保持不變。", "このプロファイルには回転と押し込みのカスタム割り当てがあります。プリセットを選ぶまで変更されません。"),
    ("rot_custom_rotation", "This profile has a custom rotation mapping. It stays untouched until you choose a preset.", "此配置有自定义的旋转映射。在您选择预设前保持不变。", "此設定檔有自訂的旋轉對應。在您選擇預設前保持不變。", "このプロファイルには回転のカスタム割り当てがあります。プリセットを選ぶまで変更されません。"),
    ("beh_shortcuts", "Application shortcuts", "应用快捷键", "應用程式快速鍵", "アプリのショートカット"),
    ("beh_macos", "macOS", "macOS", "macOS", "macOS"),
    ("beh_keystroke", "Key stroke", "按键输入", "按鍵輸入", "キー入力"),
    ("beh_app", "App", "应用", "應用程式", "アプリ"),
    ("beh_existing", "Existing setup", "现有设置", "現有設定", "現在の設定"),
    ("rot_custom_press", "This profile has a custom press mapping. It stays untouched until you choose a preset.", "此配置有自定义的按下映射。在您选择预设前保持不变。", "此設定檔有自訂的按下對應。在您選擇預設前保持不變。", "このプロファイルには押し込みのカスタム割り当てがあります。プリセットを選ぶまで変更されません。"),
];
