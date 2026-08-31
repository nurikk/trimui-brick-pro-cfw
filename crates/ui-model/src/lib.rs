use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: u16 = 1;
pub const ARTBOOK_IDENTITY: &str = "Artbook";
pub const MAX_INPUT_CHARS: usize = 80;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SystemId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct GameId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct MenuId(pub String);

impl SystemId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl GameId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl MenuId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    Home,
    Systems,
    Games,
    Search,
    Favorites,
    Recent,
    Settings,
    GameSwitcher,
    Recovery,
    Scraper(ScraperRoute),
    Wifi(WifiRoute),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScraperRoute {
    Settings,
    Game,
    BulkQueue,
    AmbiguousChoice,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiRoute {
    Scan,
    AccessPointSelection,
    HiddenNetwork,
    ManualSsid,
    PasswordEntry,
    Progress,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    Primary,
    Secondary,
    Start,
    Select,
    L1,
    R1,
    Menu,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SemanticAction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Primary,
    Secondary,
    Start,
    Select,
    JumpNextGroup,
    JumpPreviousGroup,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelpBinding {
    pub button: Button,
    pub label: String,
    pub action: Option<SemanticAction>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerHelpStrip {
    pub bindings: Vec<HelpBinding>,
}

impl Default for ControllerHelpStrip {
    fn default() -> Self {
        Self {
            bindings: vec![
                HelpBinding {
                    button: Button::Primary,
                    label: "Open".into(),
                    action: Some(SemanticAction::Primary),
                },
                HelpBinding {
                    button: Button::Secondary,
                    label: "Back".into(),
                    action: Some(SemanticAction::Secondary),
                },
                HelpBinding {
                    button: Button::Menu,
                    label: "System menu".into(),
                    action: Some(SemanticAction::Select),
                },
                HelpBinding {
                    button: Button::Start,
                    label: "Launch".into(),
                    action: Some(SemanticAction::Start),
                },
                HelpBinding {
                    button: Button::L1,
                    label: "Prev group".into(),
                    action: Some(SemanticAction::JumpPreviousGroup),
                },
                HelpBinding {
                    button: Button::R1,
                    label: "Next group".into(),
                    action: Some(SemanticAction::JumpNextGroup),
                },
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    SystemArtwork,
    SystemLogo,
    BoxArt,
    Screenshot,
    Splash,
    Fallback,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedAssetRef {
    pub id: String,
    pub kind: AssetKind,
    pub alt_text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSummary {
    pub id: SystemId,
    pub name: String,
    pub artwork: GeneratedAssetRef,
    pub logo: GeneratedAssetRef,
    pub game_count: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSummary {
    pub id: GameId,
    pub system_id: SystemId,
    pub title: String,
    pub description: String,
    pub rating: Option<u8>,
    pub release_date: Option<String>,
    pub box_art: GeneratedAssetRef,
    pub screenshot: GeneratedAssetRef,
    pub favorite: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeProjection {
    pub content_id: String,
    pub label: String,
    pub system: String,
    pub status: String,
    pub timestamp_ms: u64,
    pub screenshot: String,
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuCommand {
    Navigate(Route),
    Resume(GameId),
    OpenSystem(SystemId),
    Launch(GameId),
    ToggleFavorite(GameId),
    ApplyPreference(PreferenceChange),
    OpenScraper(ScraperRoute),
    OpenWifi(WifiRoute),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEntry {
    pub id: MenuId,
    pub label: String,
    pub command: MenuCommand,
    pub enabled: bool,
    pub disabled_reason: Option<Capability>,
    pub selected: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub index: usize,
    pub item_id: Option<MenuId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusTarget {
    Menu,
    SearchInput,
    GameList,
    Artwork,
    Modal,
    ControllerHelp,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuState {
    pub entries: Vec<MenuEntry>,
    pub selection: Selection,
}

impl MenuState {
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
            selection: Selection {
                index: 0,
                item_id: None,
            },
        }
    }

    fn sync_selection(&mut self) {
        if self.entries.is_empty() {
            self.selection = Selection {
                index: 0,
                item_id: None,
            };
            return;
        }
        self.selection.index = self.selection.index.min(self.entries.len() - 1);
        self.selection.item_id = Some(self.entries[self.selection.index].id.clone());
        for (index, entry) in self.entries.iter_mut().enumerate() {
            entry.selected = index == self.selection.index;
        }
    }

    pub fn selected(&self) -> Option<&MenuEntry> {
        self.entries.get(self.selection.index)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtworkMode {
    Large,
    Compact,
    Off,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetadataVisibility {
    Full,
    Compact,
    Hidden,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiSize {
    Automatic,
    Compact,
    Comfortable,
    Large,
    ExtraLarge,
}

impl UiSize {
    pub const fn preset_scale_percent(self) -> Option<u16> {
        match self {
            Self::Automatic => None,
            Self::Compact => Some(90),
            Self::Comfortable => Some(110),
            Self::Large => Some(125),
            Self::ExtraLarge => Some(150),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    Ink,
    Paper,
    HighContrast,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferences {
    pub artwork_mode: ArtworkMode,
    pub metadata_visibility: MetadataVisibility,
    pub ui_size: UiSize,
    pub color_scheme: ColorScheme,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            artwork_mode: ArtworkMode::Large,
            metadata_visibility: MetadataVisibility::Full,
            ui_size: UiSize::Automatic,
            color_scheme: ColorScheme::Ink,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreferenceChange {
    ArtworkMode(ArtworkMode),
    MetadataVisibility(MetadataVisibility),
    UiSize(UiSize),
    ColorScheme(ColorScheme),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AspectRatio {
    FourByThree,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutContract {
    pub aspect_ratio: AspectRatio,
    pub logical_width: u16,
    pub logical_height: u16,
    pub full_screen_menus: bool,
    pub controller_first: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameListDensity {
    Dense,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuSurface {
    FullScreen,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewContract {
    pub system_artwork_is_large: bool,
    pub game_list_density: GameListDensity,
    pub show_description: bool,
    pub show_rating: bool,
    pub show_release_date: bool,
    pub menu_surface: MenuSurface,
}

impl Default for ViewContract {
    fn default() -> Self {
        Self {
            system_artwork_is_large: true,
            game_list_density: GameListDensity::Dense,
            show_description: true,
            show_rating: true,
            show_release_date: true,
            menu_surface: MenuSurface::FullScreen,
        }
    }
}

impl Default for LayoutContract {
    fn default() -> Self {
        Self {
            aspect_ratio: AspectRatio::FourByThree,
            logical_width: 320,
            logical_height: 240,
            full_screen_menus: true,
            controller_first: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockAffordance {
    pub visible: bool,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryAffordance {
    pub visible: bool,
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformAffordances {
    pub clock: ClockAffordance,
    pub battery: BatteryAffordance,
}

impl Default for PlatformAffordances {
    fn default() -> Self {
        Self {
            clock: ClockAffordance {
                visible: true,
                value: "12:00".into(),
            },
            battery: BatteryAffordance {
                visible: true,
                percent: 100,
                charging: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Catalog,
    Favorites,
    SettingsPersistence,
    Session,
    Scraper,
    Wifi,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub catalog: bool,
    pub favorites: bool,
    pub settings_persistence: bool,
    pub session: bool,
    pub scraper: bool,
    pub wifi: bool,
}

impl Default for PlatformCapabilities {
    fn default() -> Self {
        Self {
            catalog: true,
            favorites: true,
            settings_persistence: true,
            session: true,
            scraper: false,
            wifi: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityError {
    pub capability: Capability,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModalState {
    Unavailable(CapabilityError),
    Info {
        title: String,
        message: String,
    },
    Confirm {
        title: String,
        message: String,
        command: MenuCommand,
    },
    MaskedPasswordKeyboard {
        ssid: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SplashState {
    Visible(GeneratedAssetRef),
    Ready,
    Fallback {
        reason: FallbackReason,
        asset: GeneratedAssetRef,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackReason {
    InvalidState,
    MissingContent,
    MissingCapability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Idle,
    Requested(GameId),
    Active(GameId),
    Failed { message: String },
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiFeedback {
    None,
    DisabledSelection(MenuId),
    FavoriteChanged(GameId),
    PreferenceChanged,
    GroupBoundary,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupJumpState {
    pub current: Option<String>,
    pub target: Option<String>,
    pub visible: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub contract_version: u16,
    pub identity: String,
    pub route: Route,
    pub focus: FocusTarget,
    pub menu: MenuState,
    pub settings_menu_projection: Vec<MenuEntry>,
    pub systems: Vec<SystemSummary>,
    pub resume_entries: Vec<ResumeProjection>,
    pub games: Vec<GameSummary>,
    pub selected_system: Option<SystemId>,
    pub search_query: String,
    pub preferences: UiPreferences,
    pub preview_ui_size: Option<UiSize>,
    pub layout: LayoutContract,
    pub view: ViewContract,
    pub affordances: PlatformAffordances,
    pub controller_help: ControllerHelpStrip,
    pub capabilities: PlatformCapabilities,
    pub modal: Option<ModalState>,
    pub splash: SplashState,
    pub scraper: ScraperState,
    pub wifi: WifiState,
    pub session: SessionState,
    pub feedback: UiFeedback,
    pub group_jump: GroupJumpState,
}

impl UiState {
    pub fn generated() -> Self {
        let systems = generated_systems();
        let games = generated_games();
        let route = Route::Home;
        let mut state = Self {
            contract_version: CONTRACT_VERSION,
            identity: ARTBOOK_IDENTITY.into(),
            route: route.clone(),
            focus: FocusTarget::Menu,
            menu: MenuState::empty(),
            settings_menu_projection: Vec::new(),
            systems,
            resume_entries: Vec::new(),
            games,
            selected_system: None,
            search_query: String::new(),
            preferences: UiPreferences::default(),
            preview_ui_size: None,
            layout: LayoutContract::default(),
            view: ViewContract::default(),
            affordances: PlatformAffordances::default(),
            controller_help: ControllerHelpStrip::default(),
            capabilities: PlatformCapabilities::default(),
            modal: None,
            splash: SplashState::Visible(GeneratedAssetRef {
                id: "nova8-splash".into(),
                kind: AssetKind::Splash,
                alt_text: "NOVA/8 console splash".into(),
            }),
            scraper: ScraperState::default(),
            wifi: WifiState::default(),
            session: SessionState::Idle,
            feedback: UiFeedback::None,
            group_jump: GroupJumpState::default(),
        };
        state.menu = menu_for_route(&route, &state);
        state
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeJob {
    pub game_id: GameId,
    pub progress_percent: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScraperPhase {
    Searching,
    Retrying,
    FallingBack,
    DownloadingCover,
    Publishing,
    Done,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScraperRow {
    pub game_id: GameId,
    pub title: String,
    pub provider: Option<String>,
    pub phase: ScraperPhase,
    pub fallback_transition: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScraperCounts {
    pub succeeded: u16,
    pub fallback: u16,
    pub not_found: u16,
    pub ambiguous: u16,
    pub failed: u16,
    pub cancelled: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ScraperProgress {
    pub completed: u16,
    pub total: u16,
    pub percent: u8,
    pub configured_slots: u8,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub background: bool,
    pub counts: ScraperCounts,
    pub rows: Vec<ScraperRow>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmbiguousChoice {
    pub game_id: GameId,
    pub candidates: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScraperStatus {
    Idle,
    Queued,
    Running,
    Paused,
    Cancelled,
    Complete,
    Error { message: String, blocking: bool },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScraperState {
    pub route: ScraperRoute,
    pub selected_game: Option<GameId>,
    pub queue: Vec<ScrapeJob>,
    pub progress: Option<ScraperProgress>,
    pub status: ScraperStatus,
    pub ambiguous_choice: Option<AmbiguousChoice>,
    pub selected_candidate: Option<String>,
    pub cancel_requested: bool,
}

impl Default for ScraperState {
    fn default() -> Self {
        Self {
            route: ScraperRoute::Settings,
            selected_game: None,
            queue: Vec::new(),
            progress: None,
            status: ScraperStatus::Idle,
            ambiguous_choice: None,
            selected_candidate: None,
            cancel_requested: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessPoint {
    pub ssid: String,
    pub signal_percent: u8,
    pub secured: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MaskedKeyboardRequest {
    pub ssid: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SsidEntryMode {
    Hidden,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SsidEntry {
    pub mode: SsidEntryMode,
    pub ssid: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WifiStatus {
    Unavailable,
    Idle,
    Scanning,
    AwaitingPassword,
    Connecting,
    Connected,
    Disconnected,
    Forgotten,
    Error { message: String },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiState {
    pub route: WifiRoute,
    pub selected_ssid: Option<String>,
    pub ssid_entry: Option<SsidEntry>,
    pub access_points: Vec<AccessPoint>,
    pub selected_index: usize,
    pub keyboard_request: Option<MaskedKeyboardRequest>,
    pub status: WifiStatus,
}

impl Default for WifiState {
    fn default() -> Self {
        Self {
            route: WifiRoute::Scan,
            selected_ssid: None,
            ssid_entry: None,
            access_points: Vec::new(),
            selected_index: 0,
            keyboard_request: None,
            status: WifiStatus::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Navigate(Route),
    MoveSelection(Direction),
    SelectGame(GameId),
    SetFocus(FocusTarget),
    SetSettingsMenuProjection { entries: Vec<MenuEntry> },
    ActivateSelected,
    Back,
    SetSearchQuery { query: String },
    ToggleFavorite { game_id: GameId },
    SetResumeEntries { entries: Vec<ResumeProjection> },
    SetPreference(PreferenceChange),
    PreviewUiSize(UiSize),
    ConfirmUiSizePreview,
    CancelUiSizePreview,
    TimeoutUiSizePreview,
    SetAffordances(PlatformAffordances),
    SetCapabilities(PlatformCapabilities),
    SetGroupJump(GroupJumpState),
    SetGroupBoundaryFeedback,
    FinishSplash,
    ShowFallback { reason: FallbackReason },
    ShowModal(ModalState),
    DismissModal,
    ConfirmModal,
    Launch(GameId),
    Scraper(ScraperAction),
    Wifi(WifiAction),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScraperAction {
    OpenSettings,
    OpenGame { game_id: GameId },
    QueueGame { game_id: GameId },
    OpenBulkQueue,
    OpenAmbiguousChoice(AmbiguousChoice),
    SelectAmbiguousCandidate { index: usize },
    Queue { jobs: Vec<ScrapeJob> },
    SetProgress(ScraperProgress),
    Pause,
    PauseForGate { reason: String },
    Resume,
    Cancel,
    ConfirmCancel,
    Hide,
    Close,
    InspectResults,
    Complete,
    NonBlockingError { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WifiAction {
    OpenScan,
    OpenHiddenNetwork,
    OpenManualSsid,
    EnterSsid { mode: SsidEntryMode, ssid: String },
    SetAccessPoints { access_points: Vec<AccessPoint> },
    SelectAccessPoint { index: usize },
    RequestMaskedPasswordKeyboard { ssid: String },
    Connect { ssid: String },
    Disconnect,
    Forget { ssid: String },
    Retry,
    Cancel,
    Error { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersistenceKey {
    #[serde(rename = "ui.preferences")]
    Preferences,
    #[serde(rename = "ui.favorites")]
    Favorites,
    #[serde(rename = "ui.last-route")]
    LastRoute,
    #[serde(rename = "ui.last-system")]
    LastSystem,
    #[serde(rename = "ui.last-game")]
    LastGame,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedPreferences {
    pub schema_version: u16,
    pub identity: String,
    pub key: PersistenceKey,
    pub preferences: UiPreferences,
}

impl PersistenceKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preferences => "ui.preferences",
            Self::Favorites => "ui.favorites",
            Self::LastRoute => "ui.last-route",
            Self::LastSystem => "ui.last-system",
            Self::LastGame => "ui.last-game",
        }
    }
}

pub fn reduce(state: &UiState, action: Action) -> UiState {
    let mut next = state.clone();
    next.feedback = UiFeedback::None;
    match action {
        Action::Navigate(route) => navigate(&mut next, route),
        Action::MoveSelection(direction) => move_selection(&mut next, direction),
        Action::SelectGame(game_id) => select_game(&mut next, game_id),
        Action::SetFocus(focus) => next.focus = focus,
        Action::SetSettingsMenuProjection { entries } => {
            next.settings_menu_projection = entries;
            if matches!(next.route, Route::Settings) {
                let route = next.route.clone();
                next.menu = menu_for_route(&route, &next);
            }
        }
        Action::ActivateSelected => activate_selected(&mut next),
        Action::Back => {
            if next.modal.is_some() {
                next.modal = None;
            } else {
                navigate(&mut next, Route::Home);
            }
        }
        Action::SetSearchQuery { query } => {
            next.search_query = bounded_input(&query);
            navigate(&mut next, Route::Search);
        }
        Action::ToggleFavorite { game_id } => toggle_favorite(&mut next, game_id),
        Action::SetPreference(change) => set_preference(&mut next, change),
        Action::PreviewUiSize(value) => next.preview_ui_size = Some(value),
        Action::ConfirmUiSizePreview => {
            if let Some(value) = next.preview_ui_size.take() {
                next.preferences.ui_size = value;
                next.feedback = UiFeedback::PreferenceChanged;
            }
        }
        Action::CancelUiSizePreview | Action::TimeoutUiSizePreview => {
            next.preview_ui_size = None;
        }
        Action::SetAffordances(affordances) => next.affordances = affordances,
        Action::SetResumeEntries { entries } => {
            next.resume_entries = entries;
            let route = next.route.clone();
            next.menu = menu_for_route(&route, &next);
        }
        Action::SetCapabilities(capabilities) => {
            next.capabilities = capabilities;
            let route = next.route.clone();
            next.menu = menu_for_route(&route, &next);
        }
        Action::SetGroupJump(group_jump) => next.group_jump = group_jump,
        Action::SetGroupBoundaryFeedback => next.feedback = UiFeedback::GroupBoundary,
        Action::FinishSplash => next.splash = SplashState::Ready,
        Action::ShowFallback { reason } => {
            next.route = Route::Recovery;
            next.menu = menu_for_route(&Route::Recovery, &next);
            next.splash = SplashState::Fallback {
                reason,
                asset: GeneratedAssetRef {
                    id: "nova8-fallback".into(),
                    kind: AssetKind::Fallback,
                    alt_text: "NOVA/8 safe fallback".into(),
                },
            };
        }
        Action::ShowModal(modal) => next.modal = Some(modal),
        Action::DismissModal => next.modal = None,
        Action::ConfirmModal => confirm_modal(&mut next),
        Action::Launch(game_id) => launch(&mut next, game_id),
        Action::Scraper(action) => reduce_scraper(&mut next, action),
        Action::Wifi(action) => reduce_wifi(&mut next, action),
    }
    if next.modal.is_some() {
        next.focus = FocusTarget::Modal;
    } else if matches!(next.focus, FocusTarget::Modal) {
        next.focus = FocusTarget::Menu;
    }
    next.menu.sync_selection();
    next
}

fn navigate(state: &mut UiState, route: Route) {
    match route_capability(&route) {
        Some(capability) if !capability_available(&state.capabilities, capability) => {
            state.modal = Some(unavailable(capability));
            return;
        }
        _ => {}
    }
    state.route = route.clone();
    state.menu = menu_for_route(&route, state);
}

fn move_selection(state: &mut UiState, direction: Direction) {
    if state.menu.entries.is_empty() {
        return;
    }
    let step = match direction {
        Direction::Up | Direction::Left => 0 - 1,
        Direction::Down | Direction::Right => 1,
    };
    let mut index = state.menu.selection.index as isize;
    for _ in 0..state.menu.entries.len() {
        index = (index + step).rem_euclid(state.menu.entries.len() as isize);
        if state.menu.entries[index as usize].enabled {
            state.menu.selection.index = index as usize;
            return;
        }
    }
}

fn select_game(state: &mut UiState, game_id: GameId) {
    if let Some(index) = state
        .menu
        .entries
        .iter()
        .position(|entry| entry.id.0 == game_id.0)
    {
        state.menu.selection.index = index;
    }
}

fn activate_selected(state: &mut UiState) {
    let Some(entry) = state.menu.selected().cloned() else {
        return;
    };
    if !entry.enabled {
        state.feedback = UiFeedback::DisabledSelection(entry.id);
        if let Some(capability) = entry.disabled_reason {
            state.modal = Some(unavailable(capability));
        }
        return;
    }
    execute_command(state, entry.command);
}

fn confirm_modal(state: &mut UiState) {
    let command = match state.modal.take() {
        Some(ModalState::Confirm { command, .. }) => command,
        Some(_) | None => return,
    };
    execute_command(state, command);
}

fn execute_command(state: &mut UiState, command: MenuCommand) {
    match command {
        MenuCommand::Navigate(route) => navigate(state, route),
        MenuCommand::Resume(game_id) => launch(state, game_id),
        MenuCommand::OpenSystem(system_id) => {
            state.selected_system = Some(system_id);
            navigate(state, Route::Games);
        }
        MenuCommand::Launch(game_id) => launch(state, game_id),
        MenuCommand::ToggleFavorite(game_id) => toggle_favorite(state, game_id),
        MenuCommand::ApplyPreference(change) => set_preference(state, change),
        MenuCommand::OpenScraper(route) => navigate(state, Route::Scraper(route)),
        MenuCommand::OpenWifi(route) => navigate(state, Route::Wifi(route)),
    }
}

fn toggle_favorite(state: &mut UiState, game_id: GameId) {
    if !state.capabilities.favorites {
        state.modal = Some(unavailable(Capability::Favorites));
        return;
    }
    if let Some(game) = state.games.iter_mut().find(|game| game.id == game_id) {
        game.favorite = !game.favorite;
        state.feedback = UiFeedback::FavoriteChanged(game_id);
    }
}

fn set_preference(state: &mut UiState, change: PreferenceChange) {
    if !state.capabilities.settings_persistence {
        state.modal = Some(unavailable(Capability::SettingsPersistence));
        return;
    }
    match change {
        PreferenceChange::ArtworkMode(value) => state.preferences.artwork_mode = value,
        PreferenceChange::MetadataVisibility(value) => {
            state.preferences.metadata_visibility = value
        }
        PreferenceChange::UiSize(value) => state.preferences.ui_size = value,
        PreferenceChange::ColorScheme(value) => state.preferences.color_scheme = value,
    }
    state.feedback = UiFeedback::PreferenceChanged;
}

fn launch(state: &mut UiState, game_id: GameId) {
    if !state.capabilities.session {
        state.modal = Some(unavailable(Capability::Session));
    } else if state.games.iter().any(|game| game.id == game_id)
        || state
            .resume_entries
            .iter()
            .any(|entry| entry.content_id == game_id.0)
    {
        state.session = SessionState::Requested(game_id);
    }
}

fn reduce_scraper(state: &mut UiState, action: ScraperAction) {
    if !state.capabilities.scraper {
        state.modal = Some(unavailable(Capability::Scraper));
        return;
    }
    match action {
        ScraperAction::OpenSettings => navigate(state, Route::Scraper(ScraperRoute::Settings)),
        ScraperAction::OpenGame { game_id } => {
            state.scraper.selected_game = Some(game_id);
            navigate(state, Route::Scraper(ScraperRoute::Game));
        }
        ScraperAction::QueueGame { game_id } => {
            state.scraper.selected_game = Some(game_id.clone());
            state.scraper.queue = vec![ScrapeJob {
                game_id,
                progress_percent: 0,
            }];
            state.scraper.status = ScraperStatus::Queued;
        }
        ScraperAction::OpenBulkQueue => navigate(state, Route::Scraper(ScraperRoute::BulkQueue)),
        ScraperAction::OpenAmbiguousChoice(choice) => {
            state.scraper.route = ScraperRoute::AmbiguousChoice;
            state.scraper.ambiguous_choice = Some(choice);
            state.scraper.selected_candidate = None;
            navigate(state, Route::Scraper(ScraperRoute::AmbiguousChoice));
        }
        ScraperAction::SelectAmbiguousCandidate { index } => {
            if let Some(choice) = state.scraper.ambiguous_choice.as_ref() {
                state.scraper.selected_candidate = choice.candidates.get(index).cloned();
            }
        }
        ScraperAction::Queue { jobs } => {
            state.scraper.queue = jobs;
            state.scraper.status = ScraperStatus::Queued;
        }
        ScraperAction::SetProgress(mut progress) => {
            let configured_slots = if matches!(progress.configured_slots, 1 | 2 | 4) {
                progress.configured_slots
            } else {
                state
                    .scraper
                    .progress
                    .as_ref()
                    .map_or(2, |previous| previous.configured_slots)
            };
            progress.configured_slots = configured_slots;
            if let Some(previous) = &state.scraper.progress {
                progress.total = previous.total;
                progress.completed = progress
                    .completed
                    .max(previous.completed)
                    .min(progress.total);
                progress.background |= previous.background;
            } else {
                progress.completed = progress.completed.min(progress.total);
            }
            progress.percent = if progress.total == 0 {
                0
            } else {
                ((u32::from(progress.completed) * 100) / u32::from(progress.total)) as u8
            };
            progress.rows.truncate(configured_slots as usize);
            let terminal = progress.total > 0 && progress.completed == progress.total;
            let cancelled = terminal && progress.counts.cancelled > 0;
            state.scraper.progress = Some(progress);
            state.scraper.status = if cancelled {
                ScraperStatus::Cancelled
            } else if terminal {
                ScraperStatus::Complete
            } else {
                ScraperStatus::Running
            };
            state.scraper.cancel_requested = false;
        }
        ScraperAction::Pause => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                progress.paused = true;
                progress.paused_reason = progress
                    .paused_reason
                    .clone()
                    .or_else(|| Some("user-paused".into()));
            }
            state.scraper.status = ScraperStatus::Paused;
        }
        ScraperAction::PauseForGate { reason } => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                progress.paused = true;
                progress.paused_reason = Some(scraper_gate_reason(&reason));
            }
            state.scraper.status = ScraperStatus::Paused;
        }
        ScraperAction::Resume => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                progress.paused = false;
                progress.paused_reason = None;
            }
            state.scraper.status = ScraperStatus::Running;
        }
        ScraperAction::Cancel => state.scraper.cancel_requested = true,
        ScraperAction::ConfirmCancel => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                let pending = progress.total.saturating_sub(progress.completed);
                progress.counts.cancelled = progress.counts.cancelled.saturating_add(pending);
                progress.completed = progress.total;
                progress.percent = 100;
                progress.paused = false;
                progress.paused_reason = None;
                progress.rows.clear();
            }
            state.scraper.cancel_requested = false;
            state.scraper.status = ScraperStatus::Cancelled;
        }
        ScraperAction::Hide => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                progress.background = true;
            }
            navigate(state, Route::Home);
        }
        ScraperAction::Close => {
            state.scraper.route = ScraperRoute::Settings;
            navigate(state, Route::Home);
        }
        ScraperAction::InspectResults => {
            state.scraper.route = ScraperRoute::AmbiguousChoice;
            navigate(state, Route::Scraper(ScraperRoute::AmbiguousChoice));
        }
        ScraperAction::Complete => {
            if let Some(progress) = state.scraper.progress.as_mut() {
                progress.completed = progress.total;
                progress.percent = 100;
                progress.rows.clear();
            }
            state.scraper.status = ScraperStatus::Complete;
        }
        ScraperAction::NonBlockingError { message } => {
            state.scraper.status = ScraperStatus::Error {
                message: scraper_status_message(&message),
                blocking: false,
            };
        }
    }
}

fn reduce_wifi(state: &mut UiState, action: WifiAction) {
    if !state.capabilities.wifi {
        state.modal = Some(unavailable(Capability::Wifi));
        return;
    }
    match action {
        WifiAction::OpenScan => {
            state.wifi.route = WifiRoute::Scan;
            state.wifi.status = WifiStatus::Scanning;
            state.route = Route::Wifi(WifiRoute::Scan);
        }
        WifiAction::OpenHiddenNetwork => {
            state.wifi.route = WifiRoute::HiddenNetwork;
            state.wifi.status = WifiStatus::Idle;
            state.route = Route::Wifi(WifiRoute::HiddenNetwork);
        }
        WifiAction::OpenManualSsid => {
            state.wifi.route = WifiRoute::ManualSsid;
            state.wifi.status = WifiStatus::Idle;
            state.route = Route::Wifi(WifiRoute::ManualSsid);
        }
        WifiAction::EnterSsid { mode, ssid } => {
            let ssid = bounded_input(&ssid);
            state.wifi.selected_ssid = Some(ssid.clone());
            state.wifi.ssid_entry = Some(SsidEntry { mode, ssid });
            state.wifi.keyboard_request = None;
            state.wifi.route = WifiRoute::PasswordEntry;
            state.wifi.status = WifiStatus::AwaitingPassword;
            state.route = Route::Wifi(WifiRoute::PasswordEntry);
        }
        WifiAction::SetAccessPoints { access_points } => {
            state.wifi.access_points = access_points;
            state.wifi.selected_index = 0;
            state.wifi.status = WifiStatus::Idle;
            state.wifi.route = WifiRoute::AccessPointSelection;
            state.route = Route::Wifi(WifiRoute::AccessPointSelection);
        }
        WifiAction::SelectAccessPoint { index } => {
            if index < state.wifi.access_points.len() {
                state.wifi.selected_index = index;
            }
        }
        WifiAction::RequestMaskedPasswordKeyboard { ssid } => {
            let ssid = bounded_input(&ssid);
            state.wifi.selected_ssid = Some(ssid.clone());
            state.wifi.keyboard_request = Some(MaskedKeyboardRequest { ssid: ssid.clone() });
            state.wifi.status = WifiStatus::AwaitingPassword;
            state.modal = Some(ModalState::MaskedPasswordKeyboard { ssid });
        }
        WifiAction::Connect { ssid } => {
            state.wifi.selected_ssid = Some(ssid);
            state.wifi.route = WifiRoute::Progress;
            state.wifi.status = WifiStatus::Connecting;
            state.route = Route::Wifi(WifiRoute::Progress);
        }
        WifiAction::Disconnect => state.wifi.status = WifiStatus::Disconnected,
        WifiAction::Forget { ssid } => {
            state.wifi.selected_ssid = Some(ssid);
            state.wifi.status = WifiStatus::Forgotten;
        }
        WifiAction::Retry => state.wifi.status = WifiStatus::Scanning,
        WifiAction::Cancel => state.wifi.status = WifiStatus::Cancelled,
        WifiAction::Error { message } => {
            state.wifi.route = WifiRoute::Error;
            state.wifi.status = WifiStatus::Error { message };
            state.route = Route::Wifi(WifiRoute::Error);
        }
    }
}

fn bounded_input(value: &str) -> String {
    value.chars().take(MAX_INPUT_CHARS).collect()
}

fn scraper_gate_reason(reason: &str) -> String {
    match reason {
        "network"
        | "suspended"
        | "low-battery"
        | "foreground-gameplay"
        | "storage-quota"
        | "user-paused" => reason.to_owned(),
        _ => "gate-unavailable".into(),
    }
}

fn scraper_status_message(message: &str) -> String {
    let safe: String = message
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .take(64)
        .collect();
    if safe.is_empty() {
        "scraper-error".into()
    } else {
        safe
    }
}

fn route_capability(route: &Route) -> Option<Capability> {
    match route {
        Route::Systems | Route::Games | Route::Search => Some(Capability::Catalog),
        Route::Favorites | Route::Recent => Some(Capability::Favorites),
        Route::GameSwitcher => Some(Capability::Session),
        Route::Scraper(_) => Some(Capability::Scraper),
        Route::Wifi(_) => Some(Capability::Wifi),
        _ => None,
    }
}

fn capability_available(capabilities: &PlatformCapabilities, capability: Capability) -> bool {
    match capability {
        Capability::Catalog => capabilities.catalog,
        Capability::Favorites => capabilities.favorites,
        Capability::SettingsPersistence => capabilities.settings_persistence,
        Capability::Session => capabilities.session,
        Capability::Scraper => capabilities.scraper,
        Capability::Wifi => capabilities.wifi,
    }
}

fn unavailable(capability: Capability) -> ModalState {
    ModalState::Unavailable(CapabilityError {
        capability,
        code: "capability-unavailable".into(),
        message: "This capability is unavailable in the current environment.".into(),
    })
}

fn menu_for_route(route: &Route, state: &UiState) -> MenuState {
    let previous_id = state.menu.selection.item_id.clone();
    let entries = match route {
        // Keep the first screen to the everyday loop. Optional tools live in System menu.
        Route::Home => vec![
            entry("systems", "Systems", Route::Systems, true, None),
            entry(
                "favorites",
                "Favorites",
                Route::Favorites,
                state.capabilities.favorites,
                Some(Capability::Favorites),
            ),
            entry("recent", "Recent", Route::Recent, true, None),
            entry("settings", "System menu", Route::Settings, true, None),
        ],
        Route::Systems => state
            .systems
            .iter()
            .map(|system| MenuEntry {
                id: MenuId::new(system.id.0.clone()),
                label: system.name.clone(),
                command: MenuCommand::OpenSystem(system.id.clone()),
                enabled: true,
                disabled_reason: None,
                selected: false,
            })
            .collect(),
        Route::Games | Route::Favorites | Route::Recent | Route::Search => state
            .games
            .iter()
            .filter(|game| match state.selected_system.as_ref() {
                Some(id) => id == &game.system_id,
                None => true,
            })
            .filter(|game| !matches!(route, Route::Favorites) || game.favorite)
            .filter(|game| !matches!(route, Route::Recent) || game.id.0 == "generated-game-02")
            .filter(|game| {
                state.search_query.is_empty()
                    || game
                        .title
                        .to_lowercase()
                        .contains(&state.search_query.to_lowercase())
            })
            .map(|game| MenuEntry {
                id: MenuId::new(game.id.0.clone()),
                label: game.title.clone(),
                command: MenuCommand::Launch(game.id.clone()),
                enabled: true,
                disabled_reason: None,
                selected: false,
            })
            .collect(),
        Route::Settings => state.settings_menu_projection.clone(),
        Route::GameSwitcher => {
            let mut entries = state
                .resume_entries
                .iter()
                .map(|resume| MenuEntry {
                    id: MenuId::new(resume.content_id.clone()),
                    label: resume.label.clone(),
                    command: MenuCommand::Resume(GameId::new(resume.content_id.clone())),
                    enabled: resume.status != "unavailable",
                    disabled_reason: None,
                    selected: false,
                })
                .collect::<Vec<_>>();
            entries.push(entry("home", "Home", Route::Home, true, None));
            entries
        }
        Route::Recovery => vec![entry("home", "Return to Home", Route::Home, true, None)],
        Route::Scraper(_) | Route::Wifi(_) => vec![entry("home", "Home", Route::Home, true, None)],
    };
    let index = previous_id
        .as_ref()
        .and_then(|id| menu_entry_index(&entries, id))
        .unwrap_or(0);
    let mut menu = MenuState {
        entries,
        selection: Selection {
            index,
            item_id: previous_id,
        },
    };
    menu.sync_selection();
    menu
}

fn menu_entry_index(entries: &[MenuEntry], id: &MenuId) -> Option<usize> {
    entries.iter().position(|entry| entry.id == *id)
}

fn entry(
    id: &str,
    label: &str,
    route: Route,
    enabled: bool,
    disabled_reason: Option<Capability>,
) -> MenuEntry {
    MenuEntry {
        id: MenuId::new(id),
        label: label.into(),
        command: MenuCommand::Navigate(route),
        enabled,
        disabled_reason,
        selected: false,
    }
}

fn generated_asset(id: &str, kind: AssetKind, alt_text: &str) -> GeneratedAssetRef {
    GeneratedAssetRef {
        id: id.into(),
        kind,
        alt_text: alt_text.into(),
    }
}

fn generated_systems() -> Vec<SystemSummary> {
    vec![
        SystemSummary {
            id: SystemId::new("generated-system-alpha"),
            name: "NOVA/8 HANDHELD".into(),
            artwork: generated_asset(
                "generated-art-system-alpha",
                AssetKind::SystemArtwork,
                "NOVA/8 handheld identity artwork",
            ),
            logo: generated_asset(
                "generated-logo-system-alpha",
                AssetKind::SystemLogo,
                "NOVA/8 handheld wordmark",
            ),
            game_count: 2,
        },
        SystemSummary {
            id: SystemId::new("generated-system-beta"),
            name: "LUMA STATION".into(),
            artwork: generated_asset(
                "generated-art-system-beta",
                AssetKind::SystemArtwork,
                "Luma Station identity artwork",
            ),
            logo: generated_asset(
                "generated-logo-system-beta",
                AssetKind::SystemLogo,
                "Luma Station wordmark",
            ),
            game_count: 1,
        },
    ]
}

fn generated_games() -> Vec<GameSummary> {
    vec![
        GameSummary {
            id: GameId::new("generated-game-01"),
            system_id: SystemId::new("generated-system-alpha"),
            title: "Nebula Notes".into(),
            description: "Chart a quiet starship through forgotten constellations.".into(),
            rating: Some(82),
            release_date: Some("1994-04-12".into()),
            box_art: generated_asset(
                "generated-box-art-01",
                AssetKind::BoxArt,
                "Nebula Notes original cover",
            ),
            screenshot: generated_asset(
                "generated-screen-01",
                AssetKind::Screenshot,
                "Nebula Notes gameplay screenshot",
            ),
            favorite: false,
        },
        GameSummary {
            id: GameId::new("generated-game-02"),
            system_id: SystemId::new("generated-system-alpha"),
            title: "Mirror Museum".into(),
            description: "Restore a living gallery where every reflection hides a room.".into(),
            rating: Some(74),
            release_date: Some("1996-08-03".into()),
            box_art: generated_asset(
                "generated-box-art-02",
                AssetKind::BoxArt,
                "Mirror Museum original cover",
            ),
            screenshot: generated_asset(
                "generated-screen-02",
                AssetKind::Screenshot,
                "Mirror Museum gameplay screenshot",
            ),
            favorite: true,
        },
        GameSummary {
            id: GameId::new("generated-game-03"),
            system_id: SystemId::new("generated-system-beta"),
            title: "Orbit Garden".into(),
            description: "Cultivate tidal gardens on a drifting orbital station.".into(),
            rating: None,
            release_date: None,
            box_art: generated_asset(
                "generated-box-art-03",
                AssetKind::BoxArt,
                "Orbit Garden original cover",
            ),
            screenshot: generated_asset(
                "generated-screen-03",
                AssetKind::Screenshot,
                "Orbit Garden gameplay screenshot",
            ),
            favorite: false,
        },
    ]
}

pub trait CatalogPort {
    fn systems(&self) -> &[SystemSummary];
    fn games(&self) -> &[GameSummary];
}

pub trait SettingsPort {
    fn preferences(&self) -> &UiPreferences;
    fn save_preferences(&mut self, preferences: UiPreferences) -> Result<(), PortError>;
}

pub trait FavoritesPort {
    fn is_favorite(&self, game_id: &GameId) -> bool;
    fn set_favorite(&mut self, game_id: GameId, favorite: bool) -> Result<(), PortError>;
}

pub trait SessionPort {
    fn state(&self) -> &SessionState;
    fn request_launch(&mut self, game_id: GameId) -> Result<(), PortError>;
}

pub trait PlatformCapabilitiesPort {
    fn capabilities(&self) -> &PlatformCapabilities;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct GeneratedCatalog {
    pub systems: Vec<SystemSummary>,
    pub games: Vec<GameSummary>,
}

impl Default for GeneratedCatalog {
    fn default() -> Self {
        Self {
            systems: generated_systems(),
            games: generated_games(),
        }
    }
}

impl CatalogPort for GeneratedCatalog {
    fn systems(&self) -> &[SystemSummary] {
        &self.systems
    }
    fn games(&self) -> &[GameSummary] {
        &self.games
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedSettings {
    pub value: UiPreferences,
    pub writable: bool,
}

impl Default for GeneratedSettings {
    fn default() -> Self {
        Self {
            value: UiPreferences::default(),
            writable: true,
        }
    }
}

impl SettingsPort for GeneratedSettings {
    fn preferences(&self) -> &UiPreferences {
        &self.value
    }
    fn save_preferences(&mut self, preferences: UiPreferences) -> Result<(), PortError> {
        if !self.writable {
            return Err(PortError {
                code: "settings-unavailable".into(),
                message: "Generated settings are read-only.".into(),
            });
        }
        self.value = preferences;
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct GeneratedFavorites {
    pub values: Vec<GameId>,
    pub writable: bool,
}

impl FavoritesPort for GeneratedFavorites {
    fn is_favorite(&self, game_id: &GameId) -> bool {
        self.values.iter().any(|value| value == game_id)
    }
    fn set_favorite(&mut self, game_id: GameId, favorite: bool) -> Result<(), PortError> {
        if !self.writable {
            return Err(PortError {
                code: "favorites-unavailable".into(),
                message: "Generated favorites are read-only.".into(),
            });
        }
        if favorite && !self.is_favorite(&game_id) {
            self.values.push(game_id.clone());
        }
        if !favorite {
            self.values.retain(|value| value != &game_id);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct GeneratedSession {
    pub value: SessionState,
    pub available: bool,
}

impl SessionPort for GeneratedSession {
    fn state(&self) -> &SessionState {
        &self.value
    }
    fn request_launch(&mut self, game_id: GameId) -> Result<(), PortError> {
        if !self.available {
            return Err(PortError {
                code: "session-unavailable".into(),
                message: "Generated session is unavailable.".into(),
            });
        }
        self.value = SessionState::Requested(game_id);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct GeneratedPlatformCapabilities {
    pub value: PlatformCapabilities,
}

impl PlatformCapabilitiesPort for GeneratedPlatformCapabilities {
    fn capabilities(&self) -> &PlatformCapabilities {
        &self.value
    }
}

#[cfg(test)]
mod density_preview_tests {
    use super::{reduce, Action, UiSize, UiState};

    #[test]
    fn ui_size_preview_confirm_cancel_and_timeout() {
        let state = UiState::generated();
        assert_eq!(state.preferences.ui_size, UiSize::Automatic);
        assert_eq!(
            serde_json::to_string(&UiSize::ExtraLarge).unwrap(),
            "\"extra-large\""
        );
        let preview = reduce(&state, Action::PreviewUiSize(UiSize::Large));
        assert_eq!(preview.preview_ui_size, Some(UiSize::Large));
        assert_eq!(preview.preferences.ui_size, UiSize::Automatic);
        let confirmed = reduce(&preview, Action::ConfirmUiSizePreview);
        assert_eq!(confirmed.preferences.ui_size, UiSize::Large);
        assert_eq!(confirmed.preview_ui_size, None);
        let cancelled = reduce(
            &reduce(&confirmed, Action::PreviewUiSize(UiSize::Compact)),
            Action::CancelUiSizePreview,
        );
        assert_eq!(cancelled.preferences.ui_size, UiSize::Large);
        let timed_out = reduce(
            &reduce(&cancelled, Action::PreviewUiSize(UiSize::ExtraLarge)),
            Action::TimeoutUiSizePreview,
        );
        assert_eq!(timed_out.preferences.ui_size, UiSize::Large);
    }
}
