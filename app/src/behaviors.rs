//! User-facing key behaviors and their execution mappings.
//!
//! The editor talks in terms of application shortcuts, macOS controls,
//! keystrokes, and apps. This module translates those choices into the
//! existing device slot plus optional host action without leaking that split
//! into the UI.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{Action, ControlBehavior, InputConfig, MacOsControl, Profile, Slot, SlotKind};
use crate::keycodes::{keyboard_name, mods_label};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutPreset {
    pub id: &'static str,
    /// Command label per language, in `[en, zh-Hans, zh-Hant, ja]` order —
    /// the app's own localized menu wording where it ships that localization.
    pub labels: [&'static str; 4],
    pub mods: u8,
    pub key: u16,
}

impl ShortcutPreset {
    pub fn label(&self) -> &'static str {
        self.labels[crate::i18n::lang_index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutApplication {
    pub id: &'static str,
    pub label: &'static str,
    /// Picker artwork: `simple:<slug>` brand icon or a bare Lucide name.
    pub icon: &'static str,
    pub shortcuts: &'static [ShortcutPreset],
}

// --- generated shortcut catalog begin ---
const FINDER_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", ["New Finder window", "新建访达窗口", "新增 Finder 視窗", "新規Finderウインドウ"], 0x08, 0x11),
    shortcut("new_folder", ["New folder", "新建文件夹", "新增檔案夾", "新規フォルダ"], 0x0a, 0x11),
    shortcut("go_to_folder", ["Go to folder", "前往文件夹", "前往檔案夾", "フォルダへ移動"], 0x0a, 0x0a),
    shortcut("get_info", ["Get info", "显示简介", "顯示簡介", "情報を見る"], 0x08, 0x0c),
    shortcut("quick_look", ["Quick Look", "快速查看", "快速查看", "クイックルック"], 0x00, 0x2c),
    shortcut("move_to_trash", ["Move to Trash", "移到废纸篓", "移到垃圾桶", "ゴミ箱に入れる"], 0x08, 0x2a),
    shortcut("new_tab", ["New tab", "新建标签页", "新增標籤頁", "新規タブ"], 0x08, 0x17),
    shortcut("downloads_folder", ["Open Downloads folder", "打开下载文件夹", "打開下載項目檔案夾", "ダウンロードフォルダを開く"], 0x0c, 0x0f),
    shortcut("home_folder", ["Open Home folder", "打开个人文件夹", "打開個人專屬檔案夾", "ホームフォルダを開く"], 0x0a, 0x0b),
    shortcut("duplicate", ["Duplicate", "复制", "製作副本", "複製"], 0x08, 0x07),
];

const SAFARI_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", ["New tab", "新建标签页", "新增標籤頁", "新規タブ"], 0x08, 0x17),
    shortcut("close_tab", ["Close tab", "关闭标签页", "關閉標籤頁", "タブを閉じる"], 0x08, 0x1a),
    shortcut("reopen_tab", ["Reopen last closed tab", "重新打开上一个关闭的标签页", "重新打開關閉的標籤頁", "最後に閉じたタブを開く"], 0x0a, 0x17),
    shortcut("address", ["Focus address bar", "定位到地址栏", "前往智慧型搜尋欄位", "アドレスバーを選択"], 0x08, 0x0f),
    shortcut("downloads", ["Show downloads", "显示下载项", "顯示下載項目", "ダウンロードを表示"], 0x0c, 0x0f),
    shortcut("private_window", ["New Private Window", "新建无痕浏览窗口", "新增私密瀏覽視窗", "新規プライベートウインドウ"], 0x0a, 0x11),
    shortcut("reload", ["Reload Page", "重新载入页面", "重新載入頁面", "ページを再読み込み"], 0x08, 0x15),
    shortcut("reader", ["Show Reader", "显示阅读器", "顯示閱讀器", "リーダーを表示"], 0x0a, 0x15),
    shortcut("tab_overview", ["Show Tab Overview", "显示标签页概览", "顯示標籤頁總覽", "タブの概要を表示"], 0x0a, 0x31),
    shortcut("add_reading_list", ["Add to Reading List", "添加到阅读列表", "加入閱讀列表", "リーディングリストに追加"], 0x0a, 0x07),
];

const CHROME_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_tab", ["New Tab", "新建标签页", "新增分頁", "新しいタブ"], 0x08, 0x17),
    shortcut("close_tab", ["Close Tab", "关闭标签页", "關閉分頁", "タブを閉じる"], 0x08, 0x1a),
    shortcut("reopen_tab", ["Reopen Closed Tab", "重新打开已关闭的标签页", "重新開啟已關閉的分頁", "閉じたタブを開く"], 0x0a, 0x17),
    shortcut("address", ["Open Location", "打开位置", "前往網址列", "アドレスバーを選択"], 0x08, 0x0f),
    shortcut("incognito", ["New incognito window", "新建无痕式窗口", "新增無痕式視窗", "新しいシークレットウィンドウ"], 0x0a, 0x11),
    shortcut("new_window", ["New Window", "新建窗口", "新增視窗", "新しいウィンドウ"], 0x08, 0x11),
    shortcut("reload", ["Reload This Page", "重新加载此页面", "重新載入這個頁面", "このページを再読み込み"], 0x08, 0x15),
    shortcut("bookmarks_bar", ["Always Show Bookmarks Bar", "始终显示书签栏", "一律顯示書籤列", "ブックマークバーを常に表示"], 0x0a, 0x05),
    shortcut("downloads", ["Downloads", "下载内容", "下載項目", "ダウンロード"], 0x0a, 0x0d),
    shortcut("dev_tools", ["Developer Tools", "开发者工具", "開發人員工具", "デベロッパーツール"], 0x0c, 0x0c),
];

const VSCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("command_palette", ["Command Palette", "命令面板", "命令選擇區", "コマンドパレット"], 0x0a, 0x13),
    shortcut("quick_open", ["Quick Open", "快速打开", "快速開啟", "クイックオープン"], 0x08, 0x13),
    shortcut("toggle_terminal", ["Toggle terminal", "切换终端", "切換終端機", "ターミナルの表示切り替え"], 0x01, 0x35),
    shortcut("toggle_sidebar", ["Toggle Primary Side Bar", "切换主侧栏", "切換主要側邊欄", "サイドバーの表示切り替え"], 0x08, 0x05),
    shortcut("toggle_panel", ["Toggle Panel", "切换面板", "切換面板", "パネルの表示切り替え"], 0x08, 0x0d),
    shortcut("find_in_files", ["Find in files", "在文件中查找", "在檔案中尋找", "ファイル内を検索"], 0x0a, 0x09),
    shortcut("start_debugging", ["Start Debugging", "开始调试", "開始偵錯", "デバッグの開始"], 0x00, 0x3e),
    shortcut("split_editor", ["Split Editor", "拆分编辑器", "分割編輯器", "エディターの分割"], 0x08, 0x31),
    shortcut("extensions", ["Extensions", "扩展", "擴充功能", "拡張機能"], 0x0a, 0x1b),
    shortcut("new_window", ["New window", "新建窗口", "新增視窗", "新しいウィンドウ"], 0x0a, 0x11),
];

const XCODE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("build", ["Build", "构建", "建置", "ビルド"], 0x08, 0x05),
    shortcut("run", ["Run", "运行", "執行", "実行"], 0x08, 0x15),
    shortcut("stop", ["Stop", "停止", "停止", "停止"], 0x08, 0x37),
    shortcut("test", ["Test", "测试", "測試", "テスト"], 0x08, 0x18),
    shortcut("clean_build_folder", ["Clean Build Folder", "清理构建文件夹", "清除建置檔案夾", "ビルドフォルダをクリーン"], 0x0a, 0x0e),
    shortcut("open_quickly", ["Open Quickly", "快速打开", "快速打開", "クイックオープン"], 0x0a, 0x12),
    shortcut("debug_area", ["Show Debug Area", "显示调试区域", "顯示除錯區域", "デバッグエリアを表示"], 0x0a, 0x1c),
    shortcut("navigator", ["Show/Hide Navigator", "显示/隐藏导航器", "顯示或隱藏導覽器", "ナビゲータの表示/非表示"], 0x08, 0x27),
    shortcut("inspectors", ["Show/Hide Inspectors", "显示/隐藏检查器", "顯示或隱藏檢閱器", "インスペクタの表示/非表示"], 0x0c, 0x27),
    shortcut("library", ["Show Library", "显示资源库", "顯示元件庫", "ライブラリを表示"], 0x0a, 0x0f),
];

const TERMINAL_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_window", ["New window", "新建窗口", "新增視窗", "新規ウインドウ"], 0x08, 0x11),
    shortcut("new_tab", ["New tab", "新建标签页", "新增標籤頁", "新規タブ"], 0x08, 0x17),
    shortcut("clear", ["Clear to Start", "清除到开头", "清除到開頭", "先頭までを消去"], 0x08, 0x0e),
    shortcut("close", ["Close Tab", "关闭标签页", "關閉標籤頁", "タブを閉じる"], 0x08, 0x1a),
    shortcut("find", ["Find", "查找", "尋找", "検索"], 0x08, 0x09),
    shortcut("split_pane", ["Split Pane", "拆分窗格", "分割窗格", "ペインを分割"], 0x08, 0x07),
    shortcut("next_tab", ["Next Tab", "下一个标签页", "下一個標籤頁", "次のタブ"], 0x01, 0x2b),
    shortcut("show_inspector", ["Show Inspector", "显示检查器", "顯示檢閱器", "インスペクタを表示"], 0x08, 0x0c),
    shortcut("previous_mark", ["Jump to Previous Mark", "跳到上一个标记", "跳到上一個標記", "前のマークにジャンプ"], 0x08, 0x52),
    shortcut("next_mark", ["Jump to Next Mark", "跳到下一个标记", "跳到下一個標記", "次のマークにジャンプ"], 0x08, 0x51),
];

const SLACK_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_switcher", ["Quick switcher", "快速切换器", "快速切換器", "クイックスイッチャー"], 0x08, 0x0e),
    shortcut("unreads", ["All unreads", "所有未读消息", "所有未讀訊息", "すべての未読"], 0x0a, 0x04),
    shortcut("dms", ["Direct messages", "私信", "私訊", "ダイレクトメッセージ"], 0x0a, 0x0e),
    shortcut("activity", ["Activity", "活动", "動態", "アクティビティ"], 0x0a, 0x10),
    shortcut("threads", ["Threads", "话题", "討論串", "スレッド"], 0x0a, 0x17),
    shortcut("history_back", ["Previous page", "上一页", "上一頁", "前のページに戻る"], 0x08, 0x2f),
    shortcut("history_forward", ["Next page", "下一页", "下一頁", "次のページに進む"], 0x08, 0x30),
    shortcut("preferences", ["Preferences", "偏好设置", "偏好設定", "環境設定"], 0x08, 0x36),
];

const FIGMA_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_actions", ["Quick actions", "快捷操作", "快速動作", "クイックアクション"], 0x08, 0x38),
    shortcut("move", ["Move tool", "移动工具", "移動工具", "移動ツール"], 0x00, 0x19),
    shortcut("frame", ["Frame tool", "画板工具", "框架工具", "フレームツール"], 0x00, 0x09),
    shortcut("pen", ["Pen tool", "钢笔工具", "鋼筆工具", "ペンツール"], 0x00, 0x13),
    shortcut("text", ["Text tool", "文本工具", "文字工具", "テキストツール"], 0x00, 0x17),
    shortcut("rectangle", ["Rectangle tool", "矩形工具", "矩形工具", "長方形ツール"], 0x00, 0x15),
    shortcut("components", ["Create component", "创建组件", "建立元件", "コンポーネントを作成"], 0x0c, 0x0e),
    shortcut("auto_layout", ["Add auto layout", "添加自动布局", "加入自動排版", "オートレイアウトを追加"], 0x02, 0x04),
    shortcut("toggle_ui", ["Show/hide UI", "显示/隐藏界面", "顯示或隱藏介面", "UIの表示/非表示"], 0x08, 0x31),
    shortcut("comment", ["Add comment", "添加评论", "加入註解", "コメントを追加"], 0x00, 0x06),
];

const ZOOM_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("mute", ["Mute or unmute", "静音或取消静音", "靜音或取消靜音", "ミュート/ミュート解除"], 0x0a, 0x04),
    shortcut("video", ["Start or stop video", "开启或停止视频", "開啟或停止視訊", "ビデオの開始/停止"], 0x0a, 0x19),
    shortcut("share", ["Start or stop screen share", "开始或停止共享屏幕", "開始或停止畫面分享", "画面共有の開始/停止"], 0x0a, 0x16),
    shortcut("raise_hand", ["Raise or lower hand", "举手或放下手", "舉手或放下手", "手を挙げる/下ろす"], 0x04, 0x1c),
    shortcut("record", ["Start/stop local recording", "开始/停止本地录制", "開始或停止本機錄製", "レコーディングの開始/停止"], 0x0a, 0x15),
    shortcut("chat", ["Show meeting chat", "显示会议聊天", "顯示會議聊天", "ミーティングチャットを表示"], 0x0a, 0x0b),
    shortcut("participants", ["Show participants", "显示参会者", "顯示參與者", "参加者を表示"], 0x08, 0x18),
    shortcut("gallery_view", ["Speaker or gallery view", "演讲者或画廊视图", "演講者或圖庫檢視", "スピーカー/ギャラリービュー"], 0x0a, 0x1a),
    shortcut("fullscreen", ["Enter or exit full screen", "进入或退出全屏", "進入或退出全螢幕", "全画面表示の開始/終了"], 0x0a, 0x09),
    shortcut("invite", ["Invite participants", "邀请参会者", "邀請參與者", "参加者を招待"], 0x08, 0x0c),
];

const DAVINCIRESOLVE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("add_serial_node", ["Add Serial Node", "添加串行节点", "新增串聯節點", "シリアルノードを追加"], 0x04, 0x16),
    shortcut("grab_still", ["Grab Still", "抓取静帧", "擷取靜態影格", "スチルを保存"], 0x0c, 0x0a),
    shortcut("bypass_color_grades", ["Bypass Color Grades", "旁路调色", "略過調色", "カラーグレードをバイパス"], 0x02, 0x07),
    shortcut("highlight", ["Highlight", "突出显示", "醒目標示", "ハイライト"], 0x02, 0x0b),
    shortcut("add_layer_node", ["Add Layer Node", "添加层节点", "新增圖層節點", "レイヤーノードを追加"], 0x04, 0x0f),
    shortcut("add_marker", ["Add Marker", "添加标记", "新增標記", "マーカーを追加"], 0x00, 0x10),
    shortcut("play_stop", ["Play/Stop", "播放/停止", "播放或停止", "再生/停止"], 0x00, 0x2c),
    shortcut("open_color_page", ["Color Page", "调色页面", "調色頁面", "カラーページ"], 0x02, 0x23),
    shortcut("loop_playback", ["Loop", "循环", "循環播放", "ループ"], 0x08, 0x38),
    shortcut("cinema_viewer", ["Cinema Viewer", "影院模式检视器", "劇院檢視器", "シネマビューア"], 0x08, 0x09),
];

const FINALCUTPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("blade", ["Blade", "切割", "切割", "ブレード"], 0x08, 0x05),
    shortcut("append_to_storyline", ["Append to Storyline", "追加到故事情节", "附加到故事情節", "ストーリーラインの末尾に追加"], 0x00, 0x08),
    shortcut("connect_to_storyline", ["Connect to Primary Storyline", "连接到主要故事情节", "連接到主要故事情節", "基本ストーリーラインに接続"], 0x00, 0x14),
    shortcut("insert_edit", ["Insert", "插入", "插入", "挿入"], 0x00, 0x1a),
    shortcut("overwrite_edit", ["Overwrite", "覆盖", "覆蓋", "上書き"], 0x00, 0x07),
    shortcut("add_marker", ["Add Marker", "添加标记", "新增標記", "マーカーを追加"], 0x00, 0x10),
    shortcut("select_tool", ["Select", "选择", "選擇工具", "選択"], 0x00, 0x04),
    shortcut("trim_tool", ["Trim", "修剪", "修剪工具", "トリム"], 0x00, 0x17),
    shortcut("show_retime_editor", ["Show Retime Editor", "显示重新定时编辑器", "顯示重新計時編輯器", "リタイムエディタを表示"], 0x08, 0x15),
    shortcut("play_pause", ["Play/Pause", "播放/暂停", "播放或暫停", "再生/一時停止"], 0x00, 0x2c),
];

const PREMIEREPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("add_edit", ["Add Edit", "添加编辑点", "新增編輯點", "編集点を追加"], 0x08, 0x0e),
    shortcut("add_marker", ["Add Marker", "添加标记", "新增標記", "マーカーを追加"], 0x00, 0x10),
    shortcut("insert", ["Insert", "插入", "插入", "インサート"], 0x00, 0x36),
    shortcut("overwrite", ["Overwrite", "覆盖", "覆蓋", "上書き"], 0x00, 0x37),
    shortcut("lift", ["Lift", "提升", "提升", "リフト"], 0x00, 0x33),
    shortcut("extract", ["Extract", "提取", "擷取", "抽出"], 0x00, 0x34),
    shortcut("razor_tool", ["Razor Tool", "剃刀工具", "剃刀工具", "レーザーツール"], 0x00, 0x06),
    shortcut("selection_tool", ["Selection Tool", "选择工具", "選取工具", "選択ツール"], 0x00, 0x19),
    shortcut("match_frame", ["Match Frame", "匹配帧", "符合影格", "マッチフレーム"], 0x00, 0x09),
    shortcut("export_media", ["Export Media", "导出媒体", "轉存媒體", "メディアを書き出し"], 0x08, 0x10),
];

const OBSSTUDIO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("edit_transform", ["Edit Transform", "编辑变换", "編輯變形", "トランスフォームを編集"], 0x08, 0x08),
    shortcut("fit_to_screen", ["Fit to Screen", "适应屏幕大小", "符合螢幕大小", "画面に合わせる"], 0x08, 0x09),
    shortcut("stretch_to_screen", ["Stretch to Screen", "拉伸到屏幕大小", "延展至螢幕大小", "画面に引き伸ばす"], 0x08, 0x16),
    shortcut("center_to_screen", ["Center to Screen", "居中于屏幕", "置中於螢幕", "画面中央に配置"], 0x08, 0x07),
    shortcut("reset_transform", ["Reset Transform", "重置变换", "重設變形", "トランスフォームをリセット"], 0x08, 0x15),
    shortcut("move_source_up", ["Move Source Up", "上移源", "上移來源", "ソースを上へ移動"], 0x08, 0x52),
    shortcut("move_source_down", ["Move Source Down", "下移源", "下移來源", "ソースを下へ移動"], 0x08, 0x51),
];

const PHOTOSHOP_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("free_transform", ["Free Transform", "自由变换", "任意變形", "自由変形"], 0x08, 0x17),
    shortcut("layer_via_copy", ["Layer Via Copy", "通过拷贝的图层", "拷貝的圖層", "選択範囲をコピーしたレイヤー"], 0x08, 0x0d),
    shortcut("new_layer", ["New Layer", "新建图层", "新增圖層", "新規レイヤー"], 0x0a, 0x11),
    shortcut("deselect", ["Deselect", "取消选择", "取消選取", "選択を解除"], 0x08, 0x07),
    shortcut("select_inverse", ["Inverse", "反选", "反轉選取範圍", "選択範囲を反転"], 0x0a, 0x0c),
    shortcut("increase_brush_size", ["Increase Brush Size", "增大画笔大小", "放大筆刷尺寸", "ブラシサイズを大きく"], 0x00, 0x30),
    shortcut("decrease_brush_size", ["Decrease Brush Size", "减小画笔大小", "縮小筆刷尺寸", "ブラシサイズを小さく"], 0x00, 0x2f),
    shortcut("merge_visible", ["Merge Visible", "合并可见图层", "合併可見圖層", "表示レイヤーを結合"], 0x0a, 0x08),
    shortcut("fit_on_screen", ["Fit on Screen", "按屏幕大小缩放", "顯示全頁", "画面サイズに合わせる"], 0x08, 0x27),
    shortcut("brush_tool", ["Brush Tool", "画笔工具", "筆刷工具", "ブラシツール"], 0x00, 0x05),
];

const ILLUSTRATOR_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("transform_again", ["Transform Again", "再次变换", "再次變形", "変形の繰り返し"], 0x08, 0x07),
    shortcut("group", ["Group", "编组", "群組", "グループ"], 0x08, 0x0a),
    shortcut("ungroup", ["Ungroup", "取消编组", "解散群組", "グループ解除"], 0x0a, 0x0a),
    shortcut("make_clipping_mask", ["Make Clipping Mask", "建立剪切蒙版", "建立剪裁遮色片", "クリッピングマスクを作成"], 0x08, 0x24),
    shortcut("lock_selection", ["Lock Selection", "锁定所选对象", "鎖定選取範圍", "選択オブジェクトをロック"], 0x08, 0x1f),
    shortcut("toggle_outline_mode", ["Outline", "轮廓", "外框", "アウトライン表示"], 0x08, 0x1c),
    shortcut("create_outlines", ["Create Outlines", "创建轮廓", "建立外框", "アウトラインを作成"], 0x0a, 0x12),
    shortcut("bring_to_front", ["Bring to Front", "置于顶层", "移至最前", "最前面へ"], 0x0a, 0x30),
    shortcut("send_to_back", ["Send to Back", "置于底层", "移至最後", "最背面へ"], 0x0a, 0x2f),
    shortcut("fit_artboard_in_window", ["Fit Artboard in Window", "画板适合窗口大小", "使工作區域符合視窗", "アートボードを全体表示"], 0x08, 0x27),
];

const LOGICPRO_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_stop", ["Play or Stop", "播放或停止", "播放或停止", "再生/停止"], 0x00, 0x2c),
    shortcut("record", ["Record", "录音", "錄音", "録音"], 0x00, 0x15),
    shortcut("cycle_mode", ["Cycle Mode", "循环模式", "循環模式", "サイクルモード"], 0x00, 0x06),
    shortcut("metronome", ["Metronome", "节拍器", "節拍器", "メトロノーム"], 0x00, 0x0e),
    shortcut("go_to_beginning", ["Go to Beginning", "前往开头", "前往開頭", "先頭に移動"], 0x00, 0x28),
    shortcut("show_mixer", ["Show Mixer", "显示混音器", "顯示混音器", "ミキサーを表示"], 0x00, 0x1b),
    shortcut("show_editors", ["Show Editors", "显示编辑器", "顯示編輯器", "エディタを表示"], 0x00, 0x08),
    shortcut("show_library", ["Show Library", "显示资源库", "顯示資料庫", "ライブラリを表示"], 0x00, 0x1c),
    shortcut("show_automation", ["Show Automation", "显示自动化", "顯示自動化", "オートメーションを表示"], 0x00, 0x04),
    shortcut("bounce_project", ["Bounce Project or Section", "转换项目或片段", "輸出專案或段落", "プロジェクトをバウンス"], 0x08, 0x05),
];

const BLENDER_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_edit_mode", ["Toggle Edit Mode", "切换编辑模式", "切換編輯模式", "編集モードの切り替え"], 0x00, 0x2b),
    shortcut("play_animation", ["Play Animation", "播放动画", "播放動畫", "アニメーション再生"], 0x00, 0x2c),
    shortcut("shading_pie_menu", ["Shading Pie Menu", "着色饼菜单", "著色圓餅選單", "シェーディングパイメニュー"], 0x00, 0x1d),
    shortcut("add_menu", ["Add", "添加", "新增", "追加"], 0x02, 0x04),
    shortcut("duplicate_objects", ["Duplicate Objects", "复制物体", "複製物件", "オブジェクトを複製"], 0x02, 0x07),
    shortcut("frame_all", ["Frame All", "框显全部", "框顯全部", "全体表示"], 0x00, 0x4a),
    shortcut("toggle_sidebar", ["Toggle Sidebar", "切换侧栏", "切換側邊欄", "サイドバーの表示切り替え"], 0x00, 0x11),
    shortcut("toggle_toolbar", ["Toggle Toolbar", "切换工具栏", "切換工具列", "ツールバーの表示切り替え"], 0x00, 0x17),
    shortcut("render_image", ["Render Image", "渲染图像", "算繪影像", "画像をレンダリング"], 0x00, 0x45),
    shortcut("maximize_area", ["Toggle Maximize Area", "切换区域最大化", "切換最大化區域", "エリアの最大化を切り替え"], 0x01, 0x2c),
];

const ITERM2_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("split_vertically", ["Split Vertically", "垂直拆分窗格", "垂直分割窗格", "垂直に分割"], 0x08, 0x07),
    shortcut("split_horizontally", ["Split Horizontally", "水平拆分窗格", "水平分割窗格", "水平に分割"], 0x0a, 0x07),
    shortcut("maximize_pane", ["Maximize Active Pane", "最大化当前窗格", "最大化目前窗格", "ペインを最大化"], 0x0a, 0x28),
    shortcut("clear_buffer", ["Clear Buffer", "清除缓冲区", "清除緩衝區", "バッファを消去"], 0x08, 0x0e),
    shortcut("broadcast_input_tab", ["Broadcast Input to Tab", "广播输入到标签页", "廣播輸入到標籤頁", "タブに入力をブロードキャスト"], 0x0c, 0x0c),
    shortcut("instant_replay", ["Start Instant Replay", "开始即时回放", "開始即時重播", "インスタントリプレイを開始"], 0x0c, 0x05),
    shortcut("paste_history", ["Open Paste History", "打开粘贴历史", "開啟貼上記錄", "ペースト履歴を開く"], 0x0a, 0x0b),
    shortcut("set_mark", ["Set Mark", "设置标记", "設定標記", "マークを設定"], 0x0a, 0x10),
    shortcut("open_autocomplete", ["Open Autocomplete", "打开自动补全", "開啟自動完成", "オートコンプリートを開く"], 0x08, 0x33),
];

const INTELLIJ_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("run", ["Run", "运行", "執行", "実行"], 0x01, 0x15),
    shortcut("debug", ["Debug", "调试", "偵錯", "デバッグ"], 0x01, 0x07),
    shortcut("find_action", ["Find Action", "查找操作", "尋找動作", "アクションの検索"], 0x0a, 0x04),
    shortcut("recent_files", ["Recent Files", "最近文件", "最近的檔案", "最近のファイル"], 0x08, 0x08),
    shortcut("reformat_code", ["Reformat Code", "重新格式化代码", "重新格式化程式碼", "コードの再フォーマット"], 0x0c, 0x0f),
    shortcut("rename", ["Rename", "重命名", "重新命名", "名前の変更"], 0x02, 0x3f),
    shortcut("show_context_actions", ["Show Context Actions", "显示上下文操作", "顯示情境動作", "コンテキストアクションを表示"], 0x04, 0x28),
    shortcut("build_project", ["Build Project", "构建项目", "建置專案", "プロジェクトのビルド"], 0x08, 0x42),
    shortcut("step_over", ["Step Over", "步过", "逐步跳過", "ステップオーバー"], 0x00, 0x41),
    shortcut("hide_all_tool_windows", ["Hide All Tool Windows", "隐藏所有工具窗口", "隱藏所有工具視窗", "ツールウィンドウをすべて隠す"], 0x0a, 0x45),
];

const CURSOR_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("open_agent", ["Open Agent", "打开 Agent", "開啟 Agent", "エージェントを開く"], 0x08, 0x0c),
    shortcut("toggle_chat", ["Toggle Chat", "切换聊天", "切換聊天", "チャットの表示切り替え"], 0x08, 0x0f),
    shortcut("inline_edit", ["Inline Edit", "行内编辑", "行內編輯", "インライン編集"], 0x08, 0x0e),
    shortcut("add_selection_to_chat", ["Add Selection to Chat", "将所选内容加入聊天", "將選取內容加入聊天", "選択範囲をチャットに追加"], 0x0a, 0x0f),
    shortcut("cursor_settings", ["Cursor Settings", "Cursor 设置", "Cursor 設定", "Cursorの設定"], 0x0a, 0x0d),
    shortcut("toggle_terminal", ["Toggle Terminal", "切换终端", "切換終端機", "ターミナルの表示切り替え"], 0x01, 0x35),
    shortcut("toggle_sidebar", ["Toggle Primary Side Bar", "切换主侧栏", "切換主要側邊欄", "サイドバーの表示切り替え"], 0x08, 0x05),
    shortcut("command_palette", ["Command Palette", "命令面板", "命令選擇區", "コマンドパレット"], 0x0a, 0x13),
    shortcut("toggle_panel", ["Toggle Panel", "切换面板", "切換面板", "パネルの表示切り替え"], 0x08, 0x0d),
];

const FIREFOX_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("undo_close_tab", ["Undo Close Tab", "撤销关闭标签页", "復原關閉的分頁", "閉じたタブを元に戻す"], 0x0a, 0x17),
    shortcut("reader_view", ["Toggle Reader View", "切换阅读模式", "切換閱讀模式", "リーダービューの切り替え"], 0x0c, 0x15),
    shortcut("mute_tab", ["Mute/Unmute Audio", "静音/取消静音", "靜音或取消靜音", "音声をミュート/解除"], 0x01, 0x10),
    shortcut("private_window", ["New Private Window", "新建隐私浏览窗口", "新增隱私瀏覽視窗", "新しいプライベートウィンドウ"], 0x0a, 0x13),
    shortcut("downloads", ["Downloads", "下载", "下載項目", "ダウンロード"], 0x08, 0x0d),
    shortcut("bookmarks_sidebar", ["Bookmarks Sidebar", "书签侧边栏", "書籤側邊欄", "ブックマークサイドバー"], 0x08, 0x05),
    shortcut("history_sidebar", ["History Sidebar", "历史侧边栏", "瀏覽紀錄側邊欄", "履歴サイドバー"], 0x0a, 0x0b),
    shortcut("focus_address_bar", ["Open Location", "打开位置", "前往網址列", "アドレスバーを選択"], 0x08, 0x0f),
    shortcut("next_tab", ["Next Tab", "下一个标签页", "下一個分頁", "次のタブ"], 0x01, 0x2b),
];

const ARC_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_sidebar", ["Toggle Sidebar", "切换侧边栏", "切換側邊欄", "サイドバーの表示切り替え"], 0x08, 0x16),
    shortcut("copy_url", ["Copy URL", "复制网址", "拷貝網址", "URLをコピー"], 0x0a, 0x06),
    shortcut("little_arc", ["New Little Arc Window", "新建 Little Arc 窗口", "新增 Little Arc 視窗", "新規Little Arcウインドウ"], 0x0c, 0x11),
    shortcut("next_space", ["Next Space", "下一个空间", "下一個 Space", "次のスペース"], 0x0c, 0x4f),
    shortcut("previous_space", ["Previous Space", "上一个空间", "上一個 Space", "前のスペース"], 0x0c, 0x50),
    shortcut("next_tab", ["Next Tab", "下一个标签页", "下一個標籤頁", "次のタブ"], 0x0c, 0x51),
    shortcut("previous_tab", ["Previous Tab", "上一个标签页", "上一個標籤頁", "前のタブ"], 0x0c, 0x52),
    shortcut("restore_tab", ["Restore Closed Tab", "恢复关闭的标签页", "重新打開關閉的標籤頁", "閉じたタブを復元"], 0x0a, 0x17),
    shortcut("new_tab", ["New Tab", "新建标签页", "新增標籤頁", "新規タブ"], 0x08, 0x17),
];

const NOTES_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("new_note", ["New Note", "新建备忘录", "新增備忘錄", "新規メモ"], 0x08, 0x11),
    shortcut("checklist", ["Checklist", "核对清单", "檢查表", "チェックリスト"], 0x0a, 0x0f),
    shortcut("mark_checklist_item", ["Mark as Checked", "标记为已勾选", "標示為已勾選", "チェックを付ける"], 0x0a, 0x18),
    shortcut("format_title", ["Title", "标题", "標題", "タイトル"], 0x0a, 0x17),
    shortcut("format_heading", ["Heading", "小标题", "小標題", "見出し"], 0x0a, 0x0b),
    shortcut("format_body", ["Body", "正文", "內文", "本文"], 0x0a, 0x05),
    shortcut("insert_table", ["Table", "表格", "表格", "表"], 0x0c, 0x17),
    shortcut("search_all_notes", ["Note List Search", "备忘录列表搜索", "搜尋所有備忘錄", "メモリストを検索"], 0x0c, 0x09),
];

const MAIL_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("archive", ["Archive", "归档", "封存", "アーカイブ"], 0x09, 0x04),
    shortcut("reply", ["Reply", "回复", "回覆", "返信"], 0x08, 0x15),
    shortcut("reply_all", ["Reply All", "全部回复", "全部回覆", "全員に返信"], 0x0a, 0x15),
    shortcut("forward", ["Forward", "转发", "轉寄", "転送"], 0x0a, 0x09),
    shortcut("send", ["Send", "发送", "傳送", "送信"], 0x0a, 0x07),
    shortcut("mark_read_unread", ["Mark as Read/Unread", "标记为已读/未读", "標示為已讀或未讀", "開封済み/未開封にする"], 0x0a, 0x18),
    shortcut("flag", ["Toggle Flag", "切换旗标", "切換旗標", "フラグを切り替え"], 0x0a, 0x0f),
    shortcut("get_new_mail", ["Get New Mail", "接收新邮件", "取得新郵件", "新規メールを受信"], 0x0a, 0x11),
    shortcut("move_to_junk", ["Move to Junk", "移到垃圾邮件", "移到垃圾郵件", "迷惑メールに移動"], 0x0a, 0x0d),
    shortcut("new_message", ["New Message", "新建邮件", "新增郵件", "新規メッセージ"], 0x08, 0x11),
];

const KEYNOTE_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_slideshow", ["Play Slideshow", "播放幻灯片", "播放投影片", "スライドショーを再生"], 0x0c, 0x13),
    shortcut("presenter_notes", ["Show Presenter Notes", "显示演讲者备注", "顯示簡報者備忘稿", "発表者ノートを表示"], 0x0a, 0x13),
    shortcut("new_slide", ["New Slide", "新建幻灯片", "新增投影片", "新規スライド"], 0x0a, 0x11),
    shortcut("skip_slide", ["Skip Slide", "跳过幻灯片", "略過投影片", "スライドをスキップ"], 0x0a, 0x0b),
    shortcut("add_comment", ["Comment", "批注", "註解", "コメント"], 0x0a, 0x0e),
    shortcut("group_objects", ["Group", "编组", "群組", "グループ"], 0x0c, 0x0a),
    shortcut("ungroup_objects", ["Ungroup", "取消编组", "解散群組", "グループ解除"], 0x0e, 0x0a),
    shortcut("toggle_inspector", ["Show/Hide Inspector", "显示/隐藏检查器", "顯示或隱藏檢閱器", "インスペクタの表示/非表示"], 0x0c, 0x0c),
];

const PREVIEW_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("markup_toolbar", ["Show Markup Toolbar", "显示标记工具栏", "顯示標示工具列", "マークアップツールバーを表示"], 0x0a, 0x04),
    shortcut("highlight_text", ["Highlight Text", "高亮显示文本", "標明文字", "テキストをハイライト"], 0x09, 0x0b),
    shortcut("rotate_left", ["Rotate Left", "向左旋转", "向左旋轉", "左に回転"], 0x08, 0x0f),
    shortcut("rotate_right", ["Rotate Right", "向右旋转", "向右旋轉", "右に回転"], 0x08, 0x15),
    shortcut("thumbnails", ["Thumbnails", "缩略图", "縮覽圖", "サムネール"], 0x0c, 0x1f),
    shortcut("table_of_contents", ["Table of Contents", "目录", "目錄", "目次"], 0x0c, 0x20),
    shortcut("show_inspector", ["Show Inspector", "显示检查器", "顯示檢閱器", "インスペクタを表示"], 0x08, 0x0c),
    shortcut("zoom_to_fit", ["Zoom to Fit", "缩放以适合", "縮放至適合大小", "ウインドウに合わせる"], 0x08, 0x26),
];

const NOTION_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_sidebar", ["Toggle Sidebar", "切换侧边栏", "切換側邊欄", "サイドバーの表示切り替え"], 0x08, 0x31),
    shortcut("search", ["Search", "搜索", "搜尋", "検索"], 0x08, 0x13),
    shortcut("toggle_dark_mode", ["Toggle Dark Mode", "切换深色模式", "切換深色模式", "ダークモードの切り替え"], 0x0a, 0x0f),
    shortcut("go_back", ["Go Back", "后退", "返回上一頁", "戻る"], 0x08, 0x2f),
    shortcut("go_forward", ["Go Forward", "前进", "前往下一頁", "進む"], 0x08, 0x30),
    shortcut("new_page", ["New Page", "新建页面", "新增頁面", "新規ページ"], 0x08, 0x11),
    shortcut("new_tab", ["New Tab", "新建标签页", "新增標籤頁", "新規タブ"], 0x08, 0x17),
    shortcut("comment", ["Comment", "评论", "註解", "コメント"], 0x0a, 0x10),
    shortcut("new_window", ["New Window", "新建窗口", "新增視窗", "新規ウインドウ"], 0x0a, 0x11),
];

const OBSIDIAN_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("command_palette", ["Open Command Palette", "打开命令面板", "開啟命令面板", "コマンドパレットを開く"], 0x08, 0x13),
    shortcut("quick_switcher", ["Open Quick Switcher", "打开快速切换器", "開啟快速切換器", "クイックスイッチャーを開く"], 0x08, 0x12),
    shortcut("toggle_reading_view", ["Toggle Reading View", "切换阅读视图", "切換閱讀檢視", "閲覧ビューの切り替え"], 0x08, 0x08),
    shortcut("graph_view", ["Open Graph View", "打开关系图谱", "開啟關係圖檢視", "グラフビューを開く"], 0x08, 0x0a),
    shortcut("global_search", ["Search in All Files", "在所有文件中搜索", "搜尋所有檔案", "全ファイルを検索"], 0x0a, 0x09),
    shortcut("new_note", ["Create New Note", "新建笔记", "建立新筆記", "新規ノートを作成"], 0x08, 0x11),
    shortcut("toggle_checkbox", ["Toggle Checkbox Status", "切换复选框状态", "切換核取方塊狀態", "チェックボックスを切り替え"], 0x08, 0x0f),
    shortcut("navigate_back", ["Navigate Back", "后退", "返回上一頁", "戻る"], 0x0c, 0x50),
    shortcut("navigate_forward", ["Navigate Forward", "前进", "前往下一頁", "進む"], 0x0c, 0x4f),
];

const ONEPASSWORD_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("quick_access", ["Show Quick Access", "显示快速访问", "顯示快速存取", "クイックアクセスを表示"], 0x0a, 0x2c),
    shortcut("autofill", ["Autofill", "自动填充", "自動填寫", "自動入力"], 0x08, 0x31),
    shortcut("lock", ["Lock 1Password", "锁定 1Password", "鎖定 1Password", "1Passwordをロック"], 0x0a, 0x0f),
    shortcut("copy_password", ["Copy Password", "复制密码", "拷貝密碼", "パスワードをコピー"], 0x0a, 0x06),
    shortcut("copy_one_time_password", ["Copy One-Time Password", "复制一次性密码", "拷貝一次性密碼", "ワンタイムパスワードをコピー"], 0x0c, 0x06),
    shortcut("open_and_fill", ["Open and Fill", "打开并填充", "開啟並填寫", "開いて入力"], 0x0a, 0x09),
    shortcut("new_item", ["New Item", "新建项目", "新增項目", "新規アイテム"], 0x08, 0x11),
    shortcut("edit_item", ["Edit Item", "编辑项目", "編輯項目", "アイテムを編集"], 0x08, 0x08),
    shortcut("toggle_sidebar", ["Collapse/Expand Sidebar", "折叠/展开侧边栏", "收合或展開側邊欄", "サイドバーの表示切り替え"], 0x0a, 0x07),
];

const SPOTIFY_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_pause", ["Play/Pause", "播放/暂停", "播放或暫停", "再生/一時停止"], 0x00, 0x2c),
    shortcut("next_track", ["Next Track", "下一首", "下一首", "次の曲"], 0x08, 0x4f),
    shortcut("previous_track", ["Previous Track", "上一首", "上一首", "前の曲"], 0x08, 0x50),
    shortcut("volume_up", ["Volume Up", "调高音量", "調高音量", "音量を上げる"], 0x08, 0x52),
    shortcut("volume_down", ["Volume Down", "调低音量", "調低音量", "音量を下げる"], 0x08, 0x51),
    shortcut("toggle_shuffle", ["Shuffle", "随机播放", "隨機播放", "シャッフル"], 0x04, 0x16),
    shortcut("toggle_repeat", ["Repeat", "循环播放", "重複播放", "リピート"], 0x04, 0x15),
    shortcut("like_song", ["Like/Dislike Song", "喜欢/取消喜欢歌曲", "收藏或取消收藏歌曲", "お気に入りに追加/解除"], 0x06, 0x05),
    shortcut("go_to_queue", ["Go to Queue", "前往播放队列", "前往待播清單", "再生キューを表示"], 0x06, 0x14),
    shortcut("open_search", ["Open Search", "打开搜索", "開啟搜尋", "検索を開く"], 0x08, 0x0e),
];

const MUSIC_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("play_pause", ["Play/Pause", "播放/暂停", "播放或暫停", "再生/一時停止"], 0x00, 0x2c),
    shortcut("next_song", ["Next Song", "下一首", "下一首歌曲", "次の曲"], 0x00, 0x4f),
    shortcut("previous_song", ["Previous Song", "上一首", "上一首歌曲", "前の曲"], 0x00, 0x50),
    shortcut("volume_up", ["Turn Volume Up", "调高音量", "調高音量", "音量を上げる"], 0x08, 0x52),
    shortcut("volume_down", ["Turn Volume Down", "调低音量", "調低音量", "音量を下げる"], 0x08, 0x51),
    shortcut("miniplayer", ["MiniPlayer", "迷你播放器", "迷你播放器", "ミニプレーヤー"], 0x0a, 0x10),
    shortcut("playing_next", ["Playing Next", "接下来播放", "接下來播放", "次はこちら"], 0x0c, 0x18),
    shortcut("visualizer", ["Visualizer", "视觉效果", "視覺效果", "ビジュアライザ"], 0x08, 0x17),
    shortcut("equalizer", ["Equalizer", "均衡器", "等化器", "イコライザ"], 0x0c, 0x08),
];

const DISCORD_SHORTCUTS: &[ShortcutPreset] = &[
    shortcut("toggle_mute", ["Toggle Mute", "切换静音", "切換靜音", "ミュートの切り替え"], 0x0a, 0x10),
    shortcut("toggle_deafen", ["Toggle Deafen", "切换拒听", "切換拒聽", "スピーカーミュートの切り替え"], 0x0a, 0x07),
    shortcut("quick_switcher", ["Quick Switcher", "快速切换器", "快速切換器", "クイックスイッチャー"], 0x08, 0x0e),
    shortcut("mark_server_read", ["Mark Server as Read", "将服务器标为已读", "將伺服器標為已讀", "サーバーを既読にする"], 0x02, 0x29),
    shortcut("toggle_pins", ["Toggle Pins", "切换置顶消息", "切換釘選訊息", "ピン留めの表示切り替え"], 0x08, 0x13),
    shortcut("toggle_inbox", ["Toggle Inbox", "切换收件箱", "切換收件匣", "受信ボックスの表示切り替え"], 0x08, 0x0c),
    shortcut("emoji_picker", ["Toggle Emoji Picker", "切换表情选择器", "切換表情符號選單", "絵文字ピッカーを開く"], 0x08, 0x08),
    shortcut("gif_picker", ["Toggle GIF Picker", "切换 GIF 选择器", "切換 GIF 選單", "GIFピッカーを開く"], 0x08, 0x0a),
    shortcut("upload_file", ["Upload a File", "上传文件", "上傳檔案", "ファイルをアップロード"], 0x0a, 0x18),
];

pub const APPLICATION_SHORTCUTS: &[ShortcutApplication] = &[
    ShortcutApplication {
        id: "finder",
        label: "Finder",
        icon: "app-window-mac",
        shortcuts: FINDER_SHORTCUTS,
    },
    ShortcutApplication {
        id: "safari",
        label: "Safari",
        icon: "simple:safari",
        shortcuts: SAFARI_SHORTCUTS,
    },
    ShortcutApplication {
        id: "chrome",
        label: "Google Chrome",
        icon: "simple:googlechrome",
        shortcuts: CHROME_SHORTCUTS,
    },
    ShortcutApplication {
        id: "vscode",
        label: "Visual Studio Code",
        icon: "code-xml",
        shortcuts: VSCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "xcode",
        label: "Xcode",
        icon: "simple:xcode",
        shortcuts: XCODE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "terminal",
        label: "Terminal",
        icon: "square-terminal",
        shortcuts: TERMINAL_SHORTCUTS,
    },
    ShortcutApplication {
        id: "slack",
        label: "Slack",
        icon: "hash",
        shortcuts: SLACK_SHORTCUTS,
    },
    ShortcutApplication {
        id: "figma",
        label: "Figma",
        icon: "simple:figma",
        shortcuts: FIGMA_SHORTCUTS,
    },
    ShortcutApplication {
        id: "zoom",
        label: "Zoom Workplace",
        icon: "simple:zoom",
        shortcuts: ZOOM_SHORTCUTS,
    },
    ShortcutApplication {
        id: "davinciresolve",
        label: "DaVinci Resolve",
        icon: "simple:davinciresolve",
        shortcuts: DAVINCIRESOLVE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "finalcutpro",
        label: "Final Cut Pro",
        icon: "clapperboard",
        shortcuts: FINALCUTPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "premierepro",
        label: "Premiere Pro",
        icon: "film",
        shortcuts: PREMIEREPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "obsstudio",
        label: "OBS Studio",
        icon: "simple:obsstudio",
        shortcuts: OBSSTUDIO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "photoshop",
        label: "Adobe Photoshop",
        icon: "image",
        shortcuts: PHOTOSHOP_SHORTCUTS,
    },
    ShortcutApplication {
        id: "illustrator",
        label: "Adobe Illustrator",
        icon: "pen-tool",
        shortcuts: ILLUSTRATOR_SHORTCUTS,
    },
    ShortcutApplication {
        id: "logicpro",
        label: "Logic Pro",
        icon: "music",
        shortcuts: LOGICPRO_SHORTCUTS,
    },
    ShortcutApplication {
        id: "blender",
        label: "Blender",
        icon: "simple:blender",
        shortcuts: BLENDER_SHORTCUTS,
    },
    ShortcutApplication {
        id: "iterm2",
        label: "iTerm2",
        icon: "simple:iterm2",
        shortcuts: ITERM2_SHORTCUTS,
    },
    ShortcutApplication {
        id: "intellij",
        label: "IntelliJ IDEA",
        icon: "simple:intellijidea",
        shortcuts: INTELLIJ_SHORTCUTS,
    },
    ShortcutApplication {
        id: "cursor",
        label: "Cursor",
        icon: "simple:cursor",
        shortcuts: CURSOR_SHORTCUTS,
    },
    ShortcutApplication {
        id: "firefox",
        label: "Firefox",
        icon: "simple:firefox",
        shortcuts: FIREFOX_SHORTCUTS,
    },
    ShortcutApplication {
        id: "arc",
        label: "Arc",
        icon: "simple:arc",
        shortcuts: ARC_SHORTCUTS,
    },
    ShortcutApplication {
        id: "notes",
        label: "Apple Notes",
        icon: "notebook",
        shortcuts: NOTES_SHORTCUTS,
    },
    ShortcutApplication {
        id: "mail",
        label: "Apple Mail",
        icon: "mail",
        shortcuts: MAIL_SHORTCUTS,
    },
    ShortcutApplication {
        id: "keynote",
        label: "Apple Keynote",
        icon: "presentation",
        shortcuts: KEYNOTE_SHORTCUTS,
    },
    ShortcutApplication {
        id: "preview",
        label: "Apple Preview",
        icon: "file-image",
        shortcuts: PREVIEW_SHORTCUTS,
    },
    ShortcutApplication {
        id: "notion",
        label: "Notion",
        icon: "simple:notion",
        shortcuts: NOTION_SHORTCUTS,
    },
    ShortcutApplication {
        id: "obsidian",
        label: "Obsidian",
        icon: "simple:obsidian",
        shortcuts: OBSIDIAN_SHORTCUTS,
    },
    ShortcutApplication {
        id: "onepassword",
        label: "1Password",
        icon: "simple:1password",
        shortcuts: ONEPASSWORD_SHORTCUTS,
    },
    ShortcutApplication {
        id: "spotify",
        label: "Spotify",
        icon: "simple:spotify",
        shortcuts: SPOTIFY_SHORTCUTS,
    },
    ShortcutApplication {
        id: "music",
        label: "Music",
        icon: "simple:applemusic",
        shortcuts: MUSIC_SHORTCUTS,
    },
    ShortcutApplication {
        id: "discord",
        label: "Discord",
        icon: "simple:discord",
        shortcuts: DISCORD_SHORTCUTS,
    },
];
// --- generated shortcut catalog end ---

const fn shortcut(
    id: &'static str,
    labels: [&'static str; 4],
    mods: u8,
    key: u16,
) -> ShortcutPreset {
    ShortcutPreset {
        id,
        labels,
        mods,
        key,
    }
}

pub fn shortcut_application(id: &str) -> Option<&'static ShortcutApplication> {
    APPLICATION_SHORTCUTS.iter().find(|app| app.id == id)
}

pub fn shortcut_preset(application: &str, id: &str) -> Option<&'static ShortcutPreset> {
    shortcut_application(application)?
        .shortcuts
        .iter()
        .find(|shortcut| shortcut.id == id)
}

pub fn shortcut_chord_label(shortcut: &ShortcutPreset) -> String {
    let mods = mods_label(shortcut.mods);
    let key = keyboard_name(shortcut.key).unwrap_or("Unknown key");
    if mods.is_empty() {
        key.to_string()
    } else {
        format!("{mods} + {key}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacOsPreset {
    pub command: MacOsControl,
    pub label: &'static str,
    pub detail: &'static str,
}

pub const MACOS_PRESETS: &[MacOsPreset] = &[
    macos(
        MacOsControl::BrightnessUp,
        "Brightness up",
        "Increase the built-in display brightness.",
    ),
    macos(
        MacOsControl::BrightnessDown,
        "Brightness down",
        "Decrease the built-in display brightness.",
    ),
    macos(
        MacOsControl::MissionControl,
        "Mission Control",
        "Show all open windows and spaces.",
    ),
    macos(
        MacOsControl::Applications,
        "Applications / Launchpad",
        "Show the macOS applications view.",
    ),
    macos(
        MacOsControl::Search,
        "Spotlight Search",
        "Open or close Spotlight.",
    ),
    macos(
        MacOsControl::Dictation,
        "Dictation",
        "Start or stop macOS Dictation.",
    ),
    macos(
        MacOsControl::Globe,
        "Globe / Fn",
        "Use the native Globe key for input switching and Globe shortcuts.",
    ),
    macos(
        MacOsControl::LockScreen,
        "Lock screen",
        "Lock the current macOS session.",
    ),
    macos(MacOsControl::Sleep, "Sleep", "Put this Mac to sleep."),
    macos(
        MacOsControl::VolumeUp,
        "Volume up",
        "Increase the system output volume.",
    ),
    macos(
        MacOsControl::VolumeDown,
        "Volume down",
        "Decrease the system output volume.",
    ),
    macos(MacOsControl::Mute, "Mute", "Toggle system audio mute."),
    macos(
        MacOsControl::PlayPause,
        "Play / pause",
        "Toggle media playback.",
    ),
    macos(
        MacOsControl::NextTrack,
        "Next track",
        "Skip to the next media item.",
    ),
    macos(
        MacOsControl::PreviousTrack,
        "Previous track",
        "Return to the previous media item.",
    ),
    macos(
        MacOsControl::EmojiPicker,
        "Emoji & symbols",
        "Open or close the character picker.",
    ),
];

const fn macos(command: MacOsControl, label: &'static str, detail: &'static str) -> MacOsPreset {
    MacOsPreset {
        command,
        label,
        detail,
    }
}

pub fn macos_preset(command: MacOsControl) -> &'static MacOsPreset {
    MACOS_PRESETS
        .iter()
        .find(|preset| preset.command == command)
        .unwrap_or(&MACOS_PRESETS[0])
}

/// Apply a curated application chord. Returns false only for stale/unknown
/// catalog IDs, leaving the existing mapping untouched.
pub fn apply_application_shortcut(
    input: &mut InputConfig,
    application: &str,
    shortcut_id: &str,
) -> bool {
    let Some(shortcut) = shortcut_preset(application, shortcut_id) else {
        return false;
    };
    input.behavior = Some(ControlBehavior::ApplicationShortcut {
        application: application.to_string(),
        shortcut: shortcut_id.to_string(),
    });
    input.emitted = keyboard_slot(shortcut.mods, shortcut.key);
    input.action = Action::None;
    true
}

pub fn apply_keystroke(input: &mut InputConfig, mods: u8, key: u16) {
    input.behavior = Some(ControlBehavior::Keystroke);
    input.emitted = keyboard_slot(mods & 0x0f, key);
    input.action = Action::None;
}

pub fn apply_macos(input: &mut InputConfig, slot_index: usize, command: MacOsControl) {
    input.behavior = Some(ControlBehavior::MacOs { command });
    let (emitted, action) = match command {
        MacOsControl::BrightnessUp => (consumer_slot(0x006F), Action::None),
        MacOsControl::BrightnessDown => (consumer_slot(0x0070), Action::None),
        MacOsControl::MissionControl => (keyboard_slot(0x01, 0x52), Action::None),
        MacOsControl::Applications => {
            let target = if Path::new("/System/Applications/Apps.app").exists() {
                "/System/Applications/Apps.app"
            } else {
                "/System/Applications/Launchpad.app"
            };
            (
                host_trigger(slot_index),
                Action::Open {
                    target: target.to_string(),
                },
            )
        }
        // The documented macOS default. Users can remap Spotlight in System
        // Settings, just like any other keyboard shortcut.
        MacOsControl::Search => (keyboard_slot(0x08, 0x2C), Action::None),
        MacOsControl::Dictation => (consumer_slot(0x00D8), Action::None),
        // Apple's accessory keyboard specification assigns the native Globe
        // key to Consumer-page AC Keyboard Layout Select (0x029D).
        MacOsControl::Globe => (consumer_slot(0x029D), Action::None),
        MacOsControl::LockScreen => (keyboard_slot(0x09, 0x14), Action::None),
        MacOsControl::Sleep => (
            host_trigger(slot_index),
            Action::Run {
                command: "pmset sleepnow".to_string(),
            },
        ),
        MacOsControl::VolumeUp => (consumer_slot(0x00E9), Action::None),
        MacOsControl::VolumeDown => (consumer_slot(0x00EA), Action::None),
        MacOsControl::Mute => (consumer_slot(0x00E2), Action::None),
        MacOsControl::PlayPause => (consumer_slot(0x00CD), Action::None),
        MacOsControl::NextTrack => (consumer_slot(0x00B5), Action::None),
        MacOsControl::PreviousTrack => (consumer_slot(0x00B6), Action::None),
        MacOsControl::EmojiPicker => (keyboard_slot(0x09, 0x2C), Action::None),
    };
    input.emitted = emitted;
    input.action = action;
}

pub fn apply_app(input: &mut InputConfig, slot_index: usize, target: String) {
    input.behavior = Some(ControlBehavior::App {
        target: target.clone(),
    });
    input.emitted = host_trigger(slot_index);
    input.action = Action::Open { target };
}

pub fn behavior_is_consistent(input: &InputConfig, slot_index: usize) -> bool {
    let Some(behavior) = input.behavior.clone() else {
        return false;
    };
    let mut expected = input.clone();
    let valid = match behavior {
        ControlBehavior::ApplicationShortcut {
            application,
            shortcut,
        } => apply_application_shortcut(&mut expected, &application, &shortcut),
        ControlBehavior::MacOs { command }
            if matches!(command, MacOsControl::Applications | MacOsControl::Sleep) =>
        {
            apply_macos(&mut expected, slot_index, command);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
        ControlBehavior::MacOs { command } => {
            apply_macos(&mut expected, slot_index, command);
            true
        }
        ControlBehavior::Keystroke => {
            return input.emitted.kind == SlotKind::Keyboard && input.action == Action::None;
        }
        ControlBehavior::App { target } => {
            apply_app(&mut expected, slot_index, target);
            return hidden_trigger(input.emitted) && expected.action == input.action;
        }
    };
    valid && expected.emitted == input.emitted && expected.action == input.action
}

/// Keep every host-assisted semantic behavior on a distinct, non-printing
/// chord. A user may freely assign an old hidden chord as a normal keystroke;
/// this allocator moves the hidden trigger instead of letting one press run
/// two behaviors.
pub fn normalize_hidden_triggers(profile: &mut Profile) {
    for slot_index in 0..profile.inputs.len() {
        if !is_host_assisted(&profile.inputs[slot_index]) {
            continue;
        }
        let current = profile.inputs[slot_index].emitted;
        let collision = profile
            .inputs
            .iter()
            .enumerate()
            .any(|(other, input)| other != slot_index && input.emitted == current);
        if hidden_trigger(current) && !collision {
            continue;
        }

        if let Some(candidate) = hidden_trigger_candidates().find(|candidate| {
            profile
                .inputs
                .iter()
                .enumerate()
                .all(|(other, input)| other == slot_index || input.emitted != *candidate)
        }) {
            profile.inputs[slot_index].emitted = candidate;
        }
    }
}

fn is_host_assisted(input: &InputConfig) -> bool {
    matches!(
        input.behavior.as_ref(),
        Some(ControlBehavior::App { .. })
            | Some(ControlBehavior::MacOs {
                command: MacOsControl::Applications | MacOsControl::Sleep
            })
    )
}

fn hidden_trigger(slot: Slot) -> bool {
    slot.kind == SlotKind::Keyboard && (0x68..=0x6F).contains(&slot.code) && slot.mods & 0xF0 == 0
}

fn hidden_trigger_candidates() -> impl Iterator<Item = Slot> {
    const MOD_BANKS: [u8; 16] = [
        0x00, 0x02, 0x03, 0x01, 0x04, 0x08, 0x06, 0x0A, 0x0C, 0x05, 0x09, 0x07, 0x0B, 0x0D, 0x0E,
        0x0F,
    ];
    MOD_BANKS
        .into_iter()
        .flat_map(|mods| (0x68..=0x6F).map(move |code| keyboard_slot(mods, code)))
}

fn keyboard_slot(mods: u8, key: u16) -> Slot {
    Slot {
        kind: SlotKind::Keyboard,
        mods,
        code: key,
    }
}

fn consumer_slot(code: u16) -> Slot {
    Slot {
        kind: SlotKind::Consumer,
        mods: 0,
        code,
    }
}

/// A unique, non-printing chord for host-assisted behavior. There are eight
/// F13–F20 keys in each modifier bank, enough to cover all 24 input slots.
fn host_trigger(slot_index: usize) -> Slot {
    const BANK_MODS: [u8; 3] = [0x00, 0x02, 0x03];
    keyboard_slot(
        BANK_MODS[(slot_index / 8).min(2)],
        0x68 + (slot_index % 8) as u16,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledApp {
    pub name: String,
    pub path: String,
}

/// Discover launchable macOS app bundles from the normal user-facing roots.
/// Exact paths are persisted because two app bundles can share a bundle ID.
pub fn installed_apps() -> Vec<InstalledApp> {
    let mut paths = BTreeSet::new();
    for root in application_roots() {
        collect_app_bundles(&root, 0, 3, &mut paths);
    }

    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        by_name.entry(stem.to_string()).or_default().push(path);
    }

    let mut apps = Vec::new();
    for (name, mut paths) in by_name {
        paths.sort();
        let duplicated = paths.len() > 1;
        for path in paths {
            let display_name = if duplicated {
                let parent = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .unwrap_or("Applications");
                format!("{name} — {parent}")
            } else {
                name.clone()
            };
            apps.push(InstalledApp {
                name: display_name,
                path: path.to_string_lossy().into_owned(),
            });
        }
    }
    apps
}

fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn collect_app_bundles(
    directory: &Path,
    depth: usize,
    max_depth: usize,
    found: &mut BTreeSet<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let is_app = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        if is_app {
            found.insert(path);
        } else if depth < max_depth {
            collect_app_bundles(&path, depth + 1, max_depth, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_codex_profile;
    use std::collections::HashSet;

    #[test]
    fn shortcut_catalog_is_well_formed() {
        let mut app_ids = HashSet::new();
        for app in APPLICATION_SHORTCUTS {
            assert!(app_ids.insert(app.id), "duplicate app id {}", app.id);
            assert!(!app.shortcuts.is_empty(), "{} has no shortcuts", app.id);
            let mut ids = HashSet::new();
            let mut chords = HashSet::new();
            for preset in app.shortcuts {
                assert!(
                    ids.insert(preset.id),
                    "{}: duplicate shortcut id {}",
                    app.id,
                    preset.id
                );
                assert!(
                    chords.insert((preset.mods, preset.key)),
                    "{}: duplicate chord on {}",
                    app.id,
                    preset.id
                );
                assert!(
                    keyboard_name(preset.key).is_some(),
                    "{}.{}: unknown key usage {:#04x}",
                    app.id,
                    preset.id,
                    preset.key
                );
                assert_eq!(
                    preset.mods & !0x0f,
                    0,
                    "{}.{}: chords use left-hand modifier bits only",
                    app.id,
                    preset.id
                );
                for label in preset.labels {
                    assert!(
                        !label.trim().is_empty(),
                        "{}.{}: empty label",
                        app.id,
                        preset.id
                    );
                    assert!(
                        label.chars().count() <= 34,
                        "{}.{}: label too long: {label}",
                        app.id,
                        preset.id
                    );
                }
            }
        }
    }

    #[test]
    fn shortcut_catalog_icons_resolve() {
        for app in APPLICATION_SHORTCUTS {
            if let Some(slug) = app.icon.strip_prefix("simple:") {
                assert!(
                    crate::simple_icons::find(slug).is_some(),
                    "{}: unknown Simple Icons slug {slug}",
                    app.id
                );
            } else {
                assert!(
                    crate::lucide::icon_char(app.icon).is_some(),
                    "{}: unknown Lucide icon {}",
                    app.id,
                    app.icon
                );
            }
        }
    }

    /// Saved profiles reference (application, shortcut) ids and re-derive the
    /// emitted chord from this catalog — these historical pairs must keep
    /// producing identical bytes forever.
    #[test]
    fn legacy_shortcut_ids_keep_their_chords() {
        #[rustfmt::skip]
        const FROZEN: &[(&str, &str, u8, u16)] = &[
            ("finder", "new_window", 0x08, 0x11), ("finder", "new_folder", 0x0A, 0x11),
            ("finder", "go_to_folder", 0x0A, 0x0A), ("finder", "get_info", 0x08, 0x0C),
            ("finder", "quick_look", 0x00, 0x2C), ("finder", "move_to_trash", 0x08, 0x2A),
            ("safari", "new_tab", 0x08, 0x17), ("safari", "close_tab", 0x08, 0x1A),
            ("safari", "reopen_tab", 0x0A, 0x17), ("safari", "address", 0x08, 0x0F),
            ("safari", "downloads", 0x0C, 0x0F),
            ("chrome", "new_tab", 0x08, 0x17), ("chrome", "close_tab", 0x08, 0x1A),
            ("chrome", "reopen_tab", 0x0A, 0x17), ("chrome", "address", 0x08, 0x0F),
            ("chrome", "incognito", 0x0A, 0x11),
            ("vscode", "command_palette", 0x0A, 0x13), ("vscode", "quick_open", 0x08, 0x13),
            ("vscode", "new_window", 0x0A, 0x11), ("vscode", "toggle_terminal", 0x01, 0x35),
            ("vscode", "find_in_files", 0x0A, 0x09),
            ("xcode", "build", 0x08, 0x05), ("xcode", "run", 0x08, 0x15),
            ("xcode", "test", 0x08, 0x18), ("xcode", "stop", 0x08, 0x37),
            ("xcode", "open_quickly", 0x0A, 0x12),
            ("terminal", "new_window", 0x08, 0x11), ("terminal", "new_tab", 0x08, 0x17),
            ("terminal", "clear", 0x08, 0x0E), ("terminal", "close", 0x08, 0x1A),
            ("terminal", "find", 0x08, 0x09),
            ("slack", "quick_switcher", 0x08, 0x0E), ("slack", "preferences", 0x08, 0x36),
            ("slack", "threads", 0x0A, 0x17), ("slack", "history_back", 0x08, 0x2F),
            ("slack", "history_forward", 0x08, 0x30),
            ("figma", "quick_actions", 0x08, 0x38), ("figma", "frame", 0x00, 0x09),
            ("figma", "pen", 0x00, 0x13), ("figma", "text", 0x00, 0x17),
            ("figma", "components", 0x0C, 0x0E),
            ("zoom", "mute", 0x0A, 0x04), ("zoom", "video", 0x0A, 0x19),
            ("zoom", "share", 0x0A, 0x16), ("zoom", "participants", 0x08, 0x18),
            ("zoom", "chat", 0x0A, 0x0B),
        ];
        for (app, id, mods, key) in FROZEN {
            let preset = shortcut_preset(app, id)
                .unwrap_or_else(|| panic!("{app}.{id} vanished from the catalog"));
            assert_eq!(
                (preset.mods, preset.key),
                (*mods, *key),
                "{app}.{id} chord changed"
            );
        }
    }

    fn sample_input() -> InputConfig {
        InputConfig {
            label: "Key".to_string(),
            icon: "star".to_string(),
            behavior: None,
            emitted: Slot::default(),
            action: Action::None,
        }
    }

    #[test]
    fn application_shortcut_compiles_to_one_saved_chord() {
        let mut input = sample_input();
        assert!(apply_application_shortcut(
            &mut input,
            "vscode",
            "command_palette"
        ));
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Keyboard,
                mods: 0x0A,
                code: 0x13,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::ApplicationShortcut {
                application: "vscode".to_string(),
                shortcut: "command_palette".to_string(),
            })
        );
    }

    #[test]
    fn host_app_behavior_gets_a_unique_non_printing_trigger() {
        let mut first = sample_input();
        let mut touch = sample_input();
        apply_app(&mut first, 0, "/Applications/Finder.app".to_string());
        apply_app(&mut touch, 21, "/Applications/Music.app".to_string());
        assert_ne!(first.emitted, touch.emitted);
        assert_eq!(first.emitted.kind, SlotKind::Keyboard);
        assert_eq!(touch.emitted.kind, SlotKind::Keyboard);
    }

    #[test]
    fn macos_sleep_keeps_semantics_above_its_execution_mapping() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Sleep);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Sleep
            })
        );
        assert!(matches!(input.action, Action::Run { .. }));
    }

    #[test]
    fn macos_globe_uses_apples_native_consumer_usage() {
        let mut input = sample_input();
        apply_macos(&mut input, 4, MacOsControl::Globe);
        assert_eq!(
            input.emitted,
            Slot {
                kind: SlotKind::Consumer,
                mods: 0,
                code: 0x029D,
            }
        );
        assert_eq!(input.action, Action::None);
        assert_eq!(
            input.behavior,
            Some(ControlBehavior::MacOs {
                command: MacOsControl::Globe
            })
        );
        assert!(behavior_is_consistent(&input, 4));
    }

    #[test]
    fn direct_chord_collision_reallocates_only_the_hidden_trigger() {
        let mut profile = default_codex_profile();
        apply_app(
            &mut profile.inputs[0],
            0,
            "/Applications/Finder.app".to_string(),
        );
        let hidden_before = profile.inputs[0].emitted;
        apply_keystroke(
            &mut profile.inputs[1],
            hidden_before.mods,
            hidden_before.code,
        );
        let direct = profile.inputs[1].emitted;

        normalize_hidden_triggers(&mut profile);

        assert_eq!(profile.inputs[1].emitted, direct);
        assert_ne!(profile.inputs[0].emitted, direct);
        assert!(behavior_is_consistent(&profile.inputs[0], 0));
        assert!(behavior_is_consistent(&profile.inputs[1], 1));
    }
}
