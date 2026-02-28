use serde::Deserialize;

/// Google Drive MIME type for folders.
pub const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// A single item (file or folder) from Google Drive.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[allow(dead_code)] // deserialized from API, used for display
    pub size: Option<String>,
}

/// Response from Google Drive Files.list API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveListResponse {
    #[serde(default)]
    pub files: Vec<DriveItem>,
    pub next_page_token: Option<String>,
}

/// Error response body from Google Drive API.
#[derive(Debug, Deserialize)]
pub struct DriveErrorResponse {
    pub error: DriveErrorBody,
}

#[derive(Debug, Deserialize)]
pub struct DriveErrorBody {
    #[serde(default)]
    pub errors: Vec<DriveErrorDetail>,
    #[allow(dead_code)] // deserialized for serde_json parsing
    pub code: u16,
    pub message: String, // used in parse_drive_error_reason()
}

#[derive(Debug, Deserialize)]
pub struct DriveErrorDetail {
    #[allow(dead_code)] // deserialized for serde_json parsing
    pub domain: String,
    pub reason: String,
    #[allow(dead_code)] // deserialized for serde_json parsing
    pub message: String,
}

impl DriveItem {
    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
    }

    pub fn is_flac(&self) -> bool {
        self.mime_type == "audio/flac" || self.mime_type == "audio/x-flac"
    }

    #[allow(dead_code)] // will be used for .txt sidecar detection
    pub fn is_text(&self) -> bool {
        self.mime_type == "text/plain"
    }
}
