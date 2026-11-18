use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    pub ask_before_close: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            ask_before_close: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferencesUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask_before_close: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiAssistantProvider {
    #[default]
    Openai,
    Openrouter,
    Groq,
    Custom,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantSettings {
    pub enabled: bool,
    pub onboarding_completed: bool,
    pub provider: AiAssistantProvider,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantSettingsUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onboarding_completed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AiAssistantProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAiAssistantSettings {
    pub enabled: bool,
    pub onboarding_completed: bool,
    pub provider: AiAssistantProvider,
    pub model: String,
    pub base_url: String,
    pub has_api_key: bool,
    pub censored_api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantClientContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    pub route: String,
    pub page_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantAction {
    pub id: String,
    pub server_id: String,
    pub label: String,
    pub requires_confirmation: bool,
    #[serde(flatten)]
    pub kind: AiAssistantActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AiAssistantActionKind {
    #[serde(rename = "setServerProperty")]
    SetServerProperty { key: String, value: String },
    #[serde(rename = "writeServerFile", rename_all = "camelCase")]
    WriteServerFile {
        relative_path: String,
        content: String,
    },
    #[serde(rename = "sendServerCommand")]
    SendServerCommand { command: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantChatRequest {
    pub messages: Vec<AiAssistantMessage>,
    pub context: AiAssistantClientContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantChatResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<AiAssistantAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAssistantActionResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_action: Option<AiAssistantAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ServerRuntimeStatus {
    Starting,
    Online,
    Stopping,
    #[default]
    Offline,
    Idle,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRecord {
    pub id: String,
    pub name: String,
    pub framework: String,
    pub version: String,
    #[serde(rename = "ramMB", default, deserialize_with = "lenient_u32")]
    pub ram_mb: u32,
    #[serde(default)]
    pub status: ServerRuntimeStatus,
    #[serde(default)]
    pub players: String,
    #[serde(default)]
    pub created_at: String,
    pub server_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eula_accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jar_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<ProcessPriority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_config: Option<BackupConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_ngrok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ngrok_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analytics_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_restart: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restart_times: Vec<String>,
    #[serde(default)]
    pub start_on_launch: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ServerRecord {
    pub fn auto_restart_enabled(&self) -> bool {
        self.auto_restart.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupConfig {
    pub enabled: bool,
    #[serde(serialize_with = "whole_number")]
    pub interval_hours: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupType {
    Manual,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub name: String,
    pub filename: String,
    pub path: String,
    pub size: u64,
    pub created_at: String,
    #[serde(rename = "type")]
    pub kind: BackupType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupStage {
    Idle,
    Calculating,
    Archiving,
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub server_id: String,
    pub in_progress: bool,
    pub percent: f64,
    pub stage: BackupStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupProgressEvent {
    pub server_id: String,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupCompletedEvent {
    pub server_id: String,
    pub backup: BackupEntry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelBackupResult {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServerParams {
    pub name: String,
    pub framework: String,
    pub version: String,
    #[serde(rename = "ramMB")]
    pub ram_mb: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_analytics: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_plugins: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServerCreationStage {
    CreatingFolder,
    Downloading,
    WritingFiles,
    SetupJava,
    Done,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCreationProgress {
    pub stage: ServerCreationStage,
    pub message: String,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_downloaded: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServerResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub type ChangeServerFrameworkResult = CreateServerResult;
pub type ImportServerResult = CreateServerResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLineType {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub timestamp: String,
    pub text: String,
    #[serde(rename = "type")]
    pub kind: ConsoleLineType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusUpdate {
    pub server_id: String,
    pub status: ServerRuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStats {
    pub server_id: String,
    pub tps: Option<f64>,
    #[serde(rename = "memoryUsedMB")]
    pub memory_used_mb: Option<f64>,
    #[serde(rename = "memoryMaxMB")]
    pub memory_max_mb: Option<f64>,
    pub online_players: Vec<String>,
    pub player_count: u32,
    pub max_players: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProperty {
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettingsUpdate {
    #[serde(rename = "ramMB", skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<ProcessPriority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_config: Option<BackupConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_ngrok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ngrok_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analytics_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_restart: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_times: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_on_launch: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuccessResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SuccessResult {
    pub fn ok() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportServerResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadServerFileResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    #[serde(rename = "totalMemoryMB")]
    pub total_memory_mb: u64,
    #[serde(rename = "maxRamMB")]
    pub max_ram_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftVersion {
    pub id: String,
    pub stable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModrinthProjectType {
    Plugin,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModrinthSort {
    Relevance,
    Downloads,
    Updated,
    Newest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthSearchParams {
    pub query: String,
    pub project_type: ModrinthProjectType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<ModrinthSort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthSearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_side: Option<String>,
    pub downloads: u64,
    pub follows: u64,
    pub author: String,
    pub date_modified: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthSearchResult {
    pub hits: Vec<ModrinthSearchHit>,
    pub total_hits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthInstallRequest {
    pub project_id: String,
    pub project_type: ModrinthProjectType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModrinthVersionType {
    Release,
    Beta,
    Alpha,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthVersionOption {
    pub id: String,
    pub name: String,
    pub version_number: String,
    pub version_type: ModrinthVersionType,
    pub date_published: String,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthInstallEntry {
    pub project_id: String,
    pub version_id: String,
    pub file_name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub project_type: ModrinthProjectType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    pub installed_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthInstallResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ModrinthInstallEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthUpdateResult {
    pub success: bool,
    pub updated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<ModrinthInstallEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthGalleryImage {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub featured: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModrinthProjectDetails {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub followers: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_side: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki_url: Option<String>,
    pub gallery: Vec<ModrinthGalleryImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub title: String,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub update_available: bool,
    pub latest_version: String,
    pub current_version: String,
    pub release_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changelog: Option<Vec<ChangelogEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUpdateProgress {
    pub percent: f64,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadUpdateResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPlugin {
    #[serde(flatten)]
    pub source: DefaultPluginSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum DefaultPluginSource {
    #[serde(rename_all = "camelCase")]
    Modrinth {
        project_id: String,
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icon_url: Option<String>,
        added_at: String,
    },
    #[serde(rename_all = "camelCase")]
    Manual {
        file_name: String,
        size_bytes: u64,
        added_at: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDefaultPluginEntry {
    pub project_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDefaultPluginResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<DefaultPlugin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddManualDefaultPluginsResult {
    pub success: bool,
    pub added: Vec<DefaultPlugin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDefaultPluginsResult {
    pub success: bool,
    pub installed: u32,
    pub failed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPluginInstallSummary {
    pub installed: u32,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NgrokStatus {
    pub installed: bool,
    pub installing: bool,
    pub tunnel_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NgrokTunnelInfo {
    pub server_id: String,
    pub public_url: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartNgrokResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub type InstallNgrokResult = SuccessResult;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NgrokAuthtokenValidation {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NgrokInstallProgress {
    pub percent: f64,
}

pub type AnalyticsData = Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsDataResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<AnalyticsData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mb_fields_keep_ts_casing() {
        let record: ServerRecord = serde_json::from_value(json!({
            "id": "a", "name": "n", "framework": "Paper", "version": "1.21",
            "ramMB": 2048, "status": "Offline", "players": "0/20",
            "createdAt": "x", "serverPath": "/s", "custom": 1
        }))
        .unwrap();
        assert_eq!(record.ram_mb, 2048);
        let back = serde_json::to_value(&record).unwrap();
        assert_eq!(back["ramMB"], 2048);
        assert_eq!(back["custom"], 1);
        assert!(back.get("eulaAccepted").is_none());
    }

    #[test]
    fn unions_use_ts_discriminators() {
        let plugin: DefaultPlugin = serde_json::from_value(json!({
            "source": "manual", "fileName": "a.jar", "sizeBytes": 5, "addedAt": "t", "group": "g"
        }))
        .unwrap();
        assert!(matches!(plugin.source, DefaultPluginSource::Manual { .. }));
        assert_eq!(serde_json::to_value(&plugin).unwrap()["source"], "manual");

        let action: AiAssistantAction = serde_json::from_value(json!({
            "id": "1", "serverId": "s", "label": "l", "requiresConfirmation": true,
            "kind": "writeServerFile", "relativePath": "a.yml", "content": "x"
        }))
        .unwrap();
        let back = serde_json::to_value(&action).unwrap();
        assert_eq!(back["kind"], "writeServerFile");
        assert_eq!(back["relativePath"], "a.yml");
    }
}

fn lenient_u32<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let value = f64::deserialize(deserializer)?;
    Ok(value.round().clamp(0.0, u32::MAX as f64) as u32)
}

fn whole_number<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}
