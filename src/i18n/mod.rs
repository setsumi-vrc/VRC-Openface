//! Interface text. English, Japanese and Korean live in the table below; every other
//! language has its own file with the same keys (a test checks that none is missing).

use serde::{Deserialize, Serialize};

mod de;
mod es;
mod fr;
mod it;
mod pt;
mod ru;
mod zh;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    En,
    Es,
    Pt,
    Fr,
    De,
    It,
    Ru,
    Zh,
    Ja,
    Ko,
}

impl Language {
    pub const ALL: [Language; 10] = [
        Language::En,
        Language::Es,
        Language::Pt,
        Language::Fr,
        Language::De,
        Language::It,
        Language::Ru,
        Language::Zh,
        Language::Ja,
        Language::Ko,
    ];

    pub fn native_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Es => "Español",
            Language::Pt => "Português",
            Language::Fr => "Français",
            Language::De => "Deutsch",
            Language::It => "Italiano",
            Language::Ru => "Русский",
            Language::Zh => "简体中文",
            Language::Ja => "日本語",
            Language::Ko => "한국어",
        }
    }

    /// Two-letter code, also the name of its flag image.
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Es => "es",
            Language::Pt => "pt",
            Language::Fr => "fr",
            Language::De => "de",
            Language::It => "it",
            Language::Ru => "ru",
            Language::Zh => "zh",
            Language::Ja => "ja",
            Language::Ko => "ko",
        }
    }

    fn table(self) -> Option<&'static [(&'static str, &'static str)]> {
        match self {
            Language::Es => Some(es::TEXT),
            Language::Pt => Some(pt::TEXT),
            Language::Fr => Some(fr::TEXT),
            Language::De => Some(de::TEXT),
            Language::It => Some(it::TEXT),
            Language::Ru => Some(ru::TEXT),
            Language::Zh => Some(zh::TEXT),
            Language::En | Language::Ja | Language::Ko => None,
        }
    }
}

/// (key, English, Japanese, Korean)
const TEXT: &[(&str, &str, &str, &str)] = &[
    (
        "ft_hint",
        "Send face tracking to VRChat. Off sends a neutral face and marks tracking inactive.",
        "VRChatへフェイストラッキングを送信します。オフにすると無表情を送り、トラッキングを無効にします。",
        "VRChat으로 페이스 트래킹을 보냅니다. 끄면 무표정을 보내고 트래킹을 비활성화합니다.",
    ),
    ("settings", "Settings", "設定", "설정"),
    ("color_scheme", "Color scheme", "配色", "색 구성"),
    ("camera_preview", "Camera Preview", "カメラプレビュー", "카메라 미리보기"),
    ("face_landmarks", "Face Landmarks", "顔ランドマーク", "얼굴 랜드마크"),
    ("cat_ears", "Cat ears", "猫耳", "고양이 귀"),
    (
        "cat_ears_hint",
        "Drives EarLeft / EarRight (v2) from your brows: raised brows perk the ears up, lowered brows fold them back.",
        "眉の動きで EarLeft / EarRight (v2) を動かします。眉を上げると耳が立ち、下げると耳が後ろに倒れます。",
        "눈썹으로 EarLeft / EarRight (v2)를 움직입니다. 눈썹을 올리면 귀가 서고, 내리면 뒤로 젖혀집니다.",
    ),
    ("mirror", "Mirror", "反転", "좌우 반전"),
    ("refresh_cameras", "Refresh camera list", "カメラ一覧を更新", "카메라 목록 새로고침"),
    ("capture_neutral", "Capture neutral", "ニュートラルを記録", "중립 표정 저장"),
    (
        "neutral_hint",
        "Look straight ahead with a relaxed face and open eyes, then capture. Each parameter's minimum moves to its current value for this avatar.",
        "リラックスした表情で目を開けて正面を見てから記録します。このアバターの各パラメータの最小値が現在の値になります。",
        "편안한 표정으로 눈을 뜨고 정면을 본 뒤 저장하세요. 이 아바타의 각 파라미터 최솟값이 현재 값으로 바뀝니다.",
    ),
    (
        "neutral_undo",
        "Revert the last neutral capture",
        "直前のニュートラル記録を元に戻す",
        "마지막 중립 저장 되돌리기",
    ),
    ("neutral_saved", "Neutral saved", "ニュートラルを保存しました", "중립 표정을 저장했습니다"),
    (
        "neutral_rejected",
        "Keep your relaxed face visible with your eyes open, then try again.",
        "目を開けたリラックスした顔を映してから、もう一度お試しください。",
        "눈을 뜬 편안한 얼굴이 보이게 한 뒤 다시 시도하세요.",
    ),
    ("center_gaze", "Center gaze", "視線を中央に", "시선 중앙 맞추기"),
    (
        "center_gaze_hint",
        "Look straight ahead, then click to set the gaze center.",
        "正面を見てからクリックすると視線の中心を設定します。",
        "정면을 본 뒤 클릭하면 시선 중심이 설정됩니다.",
    ),
    (
        "reset_gaze",
        "Remove the saved gaze center for this avatar.",
        "このアバターの視線中心を削除します。",
        "이 아바타의 시선 중심을 삭제합니다.",
    ),
    ("gaze_saved", "Gaze center saved", "視線の中心を保存しました", "시선 중심을 저장했습니다"),
    ("hold_still", "Hold still…", "そのまま動かないでください…", "움직이지 마세요…"),
    ("search", "Search parameter…", "パラメータを検索…", "파라미터 검색…"),
    ("mediapipe", "MediaPipe", "MediaPipe", "MediaPipe"),
    ("smoothing", "Smoothing", "スムージング", "스무딩"),
    ("smoothing_off", "Off", "オフ", "끔"),
    (
        "smoothing_hint",
        "One Euro filter on the inputs before calibration. Steady values are smoothed strongly while fast movements follow with little delay.",
        "キャリブレーション前の入力にOne Euroフィルターをかけます。静止時は強く平滑化し、速い動きには少ない遅延で追従します。",
        "보정 전 입력에 One Euro 필터를 적용합니다. 멈춘 값은 강하게 부드럽게 하고 빠른 움직임은 적은 지연으로 따라갑니다.",
    ),
    ("symmetric_face", "Symmetric face", "左右対称", "좌우 대칭"),
    (
        "symmetric_hint",
        "Average both sides of the face. Sideways jaw and tongue movement stays independent.",
        "顔の左右を平均します。顎と舌の左右の動きは独立したままです。",
        "얼굴 양쪽을 평균합니다. 턱과 혀의 좌우 움직임은 독립적으로 유지됩니다.",
    ),
    ("reset", "Reset", "リセット", "초기화"),
    ("reset_rows", "Reset parameter settings", "パラメータ設定をリセット", "파라미터 설정 초기화"),
    ("tongue", "Tongue", "舌", "혀"),
    ("unused", "Unused", "未使用", "미사용"),
    ("unified", "Unified Expressions", "Unified Expressions", "Unified Expressions"),
    ("avatar", "Avatar:", "アバター:", "아바타:"),
    ("no_params", "Parameters not found", "パラメータが見つかりません", "파라미터를 찾을 수 없습니다"),
    (
        "no_vrchat",
        "Waiting for VRChat. Enable OSC in VRChat's action menu.",
        "VRChatを待っています。アクションメニューでOSCを有効にしてください。",
        "VRChat을 기다리는 중입니다. 액션 메뉴에서 OSC를 켜세요.",
    ),
    (
        "sending_default",
        "Sending FT/v2 floats for every parameter.",
        "全パラメータをFT/v2のfloatで送信中です。",
        "모든 파라미터를 FT/v2 float로 보내는 중입니다.",
    ),
    (
        "no_ft_params",
        "No face tracking parameters on this avatar",
        "このアバターにはフェイストラッキング用パラメータがありません",
        "이 아바타에는 페이스 트래킹 파라미터가 없습니다",
    ),
    (
        "linked",
        "Left and right share this calibration. Click to edit them separately.",
        "左右で同じキャリブレーションを使います。クリックで個別に編集します。",
        "왼쪽과 오른쪽이 같은 보정을 사용합니다. 클릭하면 따로 편집합니다.",
    ),
    (
        "unlinked",
        "Left and right are calibrated separately. Click to link them to this side.",
        "左右を個別にキャリブレーションしています。クリックでこちら側に揃えます。",
        "왼쪽과 오른쪽을 따로 보정합니다. 클릭하면 이쪽에 맞춥니다.",
    ),
    ("source", "Source", "入力元", "입력 소스"),
    ("model", "Model", "モデル", "모델"),
    ("mesh", "Mesh", "メッシュ", "메시"),
    ("symmetry", "Symmetry", "左右対称", "대칭"),
    ("override", "Override", "固定値", "고정값"),
    ("range", "Range", "範囲", "범위"),
    ("current_input", "current input before calibration", "キャリブレーション前の現在の入力", "보정 전 현재 입력"),
    ("calibrated", "Calibrated", "キャリブレーション済み", "보정됨"),
    ("tracking", "Tracking", "トラッキング", "트래킹"),
    ("camera", "Camera", "カメラ", "카메라"),
    ("stopped", "Tracking stopped", "トラッキング停止中", "트래킹 중지됨"),
    ("starting", "Initializing camera…", "カメラを初期化中…", "카메라 초기화 중…"),
    ("no_face", "Face not detected", "顔が検出されません", "얼굴이 감지되지 않습니다"),
    ("camera_error", "Camera unavailable", "カメラを使用できません", "카메라를 사용할 수 없습니다"),
    ("model_error", "Model error", "モデルエラー", "모델 오류"),
    ("vrchat_port", "VRChat Port", "VRChat ポート", "VRChat 포트"),
    ("in_port", "In (Receive):", "受信ポート:", "수신 포트:"),
    ("interface_scale", "Interface scale", "表示倍率", "인터페이스 배율"),
    ("idle_sleep", "Sleep when idle", "未操作時にスリープ", "유휴 시 절전"),
    (
        "idle_hint",
        "Pauses the interface after the selected time without interaction. Tracking and OSC keep running.",
        "操作がない状態が続くと画面の更新を止めます。トラッキングとOSCは動き続けます。",
        "선택한 시간 동안 조작이 없으면 화면 갱신을 멈춥니다. 트래킹과 OSC는 계속 동작합니다.",
    ),
    ("never", "Never", "しない", "안 함"),
    ("min", "min", "分", "분"),
    ("sleep_title", "Interface paused", "画面を一時停止中", "화면 일시 정지됨"),
    (
        "sleep_body",
        "Face tracking and OSC to VRChat keep running. Move the mouse or press any key to continue.",
        "フェイストラッキングとVRChatへのOSCは動作中です。マウスを動かすかキーを押すと再開します。",
        "페이스 트래킹과 VRChat OSC는 계속 동작합니다. 마우스를 움직이거나 키를 누르면 다시 시작합니다.",
    ),
    ("friendly_names", "Friendly names", "わかりやすい名前", "알기 쉬운 이름"),
    ("interpolation", "Output interpolation", "出力の補間", "출력 보간"),
    (
        "interpolation_hint",
        "Sends at 100 Hz and blends between camera frames so the avatar moves smoothly. Adds about one camera frame of latency.",
        "100Hzで送信し、カメラのフレーム間を補間してアバターを滑らかに動かします。遅延がカメラ約1フレーム分増えます。",
        "100Hz로 보내고 카메라 프레임 사이를 보간해 아바타를 부드럽게 움직입니다. 지연이 카메라 약 1프레임만큼 늘어납니다.",
    ),
    ("send_without_avatar", "Send without avatar info", "アバター情報なしでも送信", "아바타 정보 없이도 전송"),
    (
        "send_without_avatar_hint",
        "When VRChat's parameter list is unknown, send every parameter as an FT/v2 float.",
        "VRChatのパラメータ一覧がわからない場合、全パラメータをFT/v2のfloatで送ります。",
        "VRChat 파라미터 목록을 알 수 없을 때 모든 파라미터를 FT/v2 float로 보냅니다.",
    ),
    ("updates", "Updates", "アップデート", "업데이트"),
    ("channel", "Channel", "チャンネル", "채널"),
    ("stable", "Stable", "安定版", "안정판"),
    ("beta", "Beta", "ベータ", "베타"),
    ("version", "Version", "バージョン", "버전"),
    ("latest", "Latest", "最新", "최신"),
    ("check_updates", "Check updates", "アップデートを確認", "업데이트 확인"),
    ("checking", "Checking…", "確認中…", "확인 중…"),
    ("update_now", "Update now", "今すぐ更新", "지금 업데이트"),
    ("install_restart", "Install & restart", "インストールして再起動", "설치 후 재시작"),
    ("up_to_date", "Up to date", "最新です", "최신 버전입니다"),
    ("no_releases", "No releases yet", "リリースはまだありません", "아직 릴리스가 없습니다"),
    ("downloading", "Downloading…", "ダウンロード中…", "다운로드 중…"),
    ("update_failed", "Update failed", "アップデートに失敗しました", "업데이트 실패"),
    ("releases", "Releases", "リリース一覧", "릴리스"),
    ("debug_performance", "Debug performance", "パフォーマンス表示", "성능 디버그"),
    ("blur_preview", "Blur camera preview", "カメラプレビューをぼかす", "카메라 미리보기 흐리게"),
    ("processing_devices", "Processing devices", "処理デバイス", "처리 장치"),
    ("auto", "Auto", "自動", "자동"),
    ("cpu", "CPU", "CPU", "CPU"),
    ("all", "All:", "すべて:", "전체:"),
    ("camera_capture", "Camera capture", "カメラ取り込み", "카메라 캡처"),
    (
        "capture_hint",
        "Camera read time includes waiting for the next frame.",
        "カメラの読み込み時間には次のフレームを待つ時間も含まれます。",
        "카메라 읽기 시간에는 다음 프레임을 기다리는 시간도 포함됩니다.",
    ),
    (
        "enable_tongue",
        "Enable Tongue to start processing.",
        "舌を有効にすると処理が始まります。",
        "혀를 켜면 처리가 시작됩니다.",
    ),
    ("tongue_model", "Tongue model", "舌モデル", "혀 모델"),
    ("import_tongue", "Import tongue model…", "舌モデルを読み込む…", "혀 모델 가져오기…"),
    (
        "tongue_missing",
        "No tongue model. Import an ONNX tongue model to enable Tongue.",
        "舌モデルがありません。ONNXの舌モデルを読み込むと舌が有効になります。",
        "혀 모델이 없습니다. ONNX 혀 모델을 가져오면 혀가 활성화됩니다.",
    ),
    ("tongue_ready", "Tongue model installed", "舌モデルを導入済み", "혀 모델 설치됨"),
    ("resolution", "Resolution", "解像度", "해상도"),
    ("camera_fps", "Camera FPS", "カメラFPS", "카메라 FPS"),
    ("osc_host", "OSC address", "OSCアドレス", "OSC 주소"),
    ("erase", "Erase all app data", "アプリのデータをすべて消去", "앱 데이터 모두 삭제"),
    ("erase_confirm", "Click again to erase", "もう一度クリックで消去", "한 번 더 클릭하면 삭제"),
    (
        "erase_hint",
        "Delete every setting, calibration and imported model, then restart.",
        "設定、キャリブレーション、読み込んだモデルをすべて削除して再起動します。",
        "모든 설정, 보정, 가져온 모델을 삭제하고 재시작합니다.",
    ),
    ("latency", "Latency", "遅延", "지연"),
    ("averages", "1 s / 12 s average, ms", "1秒 / 12秒平均 (ms)", "1초 / 12초 평균 (ms)"),
    ("theme_black", "Black", "ブラック", "블랙"),
    ("theme_light", "Light", "ライト", "라이트"),
    ("theme_violet", "Violet", "バイオレット", "바이올렛"),
    ("theme_pink", "Pink", "ピンク", "핑크"),
    ("theme_blue", "Blue", "ブルー", "블루"),
    ("theme_green", "Green", "グリーン", "그린"),
    ("theme_custom", "Custom", "カスタム", "사용자 지정"),
    ("theme_background", "Background", "背景", "배경"),
    ("theme_panel", "Panels", "パネル", "패널"),
    ("theme_text", "Text", "文字", "글자"),
    ("theme_accent", "Accent", "アクセント", "강조"),
    (
        "mapping_docs",
        "Open the Unified Expressions parameter reference",
        "Unified Expressionsのパラメータ一覧を開く",
        "Unified Expressions 파라미터 목록 열기",
    ),
    ("osc_error", "OSC error", "OSCエラー", "OSC 오류"),
    ("receive_error", "Receive port unavailable", "受信ポートを使用できません", "수신 포트를 사용할 수 없습니다"),
    ("total_latency", "Total latency, last 12 s", "合計レイテンシ(直近12秒)", "총 지연 시간, 최근 12초"),
    ("memory", "Memory, change over 10 min", "メモリ(10分間の変化)", "메모리, 10분간 변화"),
    ("perf_camera", "Camera", "カメラ", "카메라"),
    ("perf_tracker", "Tracker", "トラッカー", "트래커"),
    ("perf_core", "Core", "コア", "코어"),
    ("perf_interface", "Interface", "画面", "인터페이스"),
    ("perf_waiting", "Waiting", "待機", "대기"),
    ("perf_queue", "Queue", "待ち", "대기열"),
    ("path_mediapipe_osc", "MediaPipe → OSC", "MediaPipe → OSC", "MediaPipe → OSC"),
    ("path_tongue_osc", "Tongue → OSC", "舌 → OSC", "혀 → OSC"),
    ("path_tables", "Tables", "表", "표"),
    ("path_preview", "Preview", "プレビュー", "미리보기"),
    ("mem_now", "Now, MiB", "現在 MiB", "현재 MiB"),
    ("mem_change", "Change", "変化", "변화"),
    ("mem_trend", "Trend, MiB/min", "傾向 MiB/分", "추세 MiB/분"),
    ("mem_private", "Private", "専用", "전용"),
    ("mem_working", "Working set", "ワーキングセット", "작업 집합"),
    ("st_read_wait", "Read + wait", "読み込み+待機", "읽기+대기"),
    ("st_convert", "Convert", "変換", "변환"),
    ("st_mirror", "Mirror", "反転", "좌우 반전"),
    ("st_mediapipe", "MediaPipe", "MediaPipe", "MediaPipe"),
    ("st_tongue", "Tongue", "舌", "혀"),
    ("st_preview", "Preview", "プレビュー", "미리보기"),
    ("st_calibration", "Calibration", "キャリブレーション", "보정"),
    ("st_mapping", "Mapping", "変換", "매핑"),
    ("st_handoff", "Hand-off", "受け渡し", "전달"),
    ("st_osc", "OSC send", "OSC送信", "OSC 전송"),
    ("st_snapshot", "Snapshot copy", "スナップショット複製", "스냅샷 복사"),
    ("st_upload", "Preview upload", "プレビュー転送", "미리보기 업로드"),
    ("st_tables", "Tables", "表", "표"),
    ("st_draw", "Draw", "描画", "그리기"),
    ("tray_open", "Open", "開く", "열기"),
    ("tray_quit", "Quit", "終了", "종료"),
];

pub fn tr(language: Language, key: &str) -> &'static str {
    let Some((_, en, ja, ko)) = TEXT.iter().find(|(k, ..)| *k == key) else {
        debug_assert!(false, "missing text {key}");
        return "?";
    };
    match language {
        Language::En => en,
        Language::Ja => ja,
        Language::Ko => ko,
        other => {
            other.table().and_then(|table| table.iter().find(|(k, _)| *k == key)).map(|(_, text)| *text).unwrap_or(en)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_text() {
        for language in Language::ALL {
            let Some(table) = language.table() else { continue };
            for (key, ..) in TEXT {
                assert!(table.iter().any(|(k, _)| k == key), "{language:?} is missing {key}");
            }
            for (key, _) in table {
                assert!(TEXT.iter().any(|(k, ..)| k == key), "{language:?} has unknown {key}");
            }
        }
    }
}
