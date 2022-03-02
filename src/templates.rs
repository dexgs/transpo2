use trillium_askama::Template;
use crate::config::*;

use std::cmp;
use std::sync::Arc;


#[derive(Template, Clone)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    app_name: Arc<String>,
    max_days: usize,
    max_hours: usize,
    max_minutes: usize,
    max_upload_size: usize
}

impl From<&TranspoConfig> for IndexTemplate {
    fn from(config: &TranspoConfig) -> Self {
        let app_name = Arc::new(config.app_name.clone());

        let max_days = cmp::max(config.max_upload_age_minutes / (24 * 60) - 1, 0);

        let max_hours = if max_days > 0 {
            23
        } else {
            cmp::max(config.max_upload_age_minutes / 60 - 1, 0)
        };

        let max_minutes = if max_hours > 0 {
            59
        } else {
            config.max_upload_age_minutes
        };

        Self {
            app_name: app_name,
            max_days: max_days,
            max_hours: max_hours,
            max_minutes: max_minutes,
            max_upload_size: config.max_upload_size_bytes
        }
    }
}

#[derive(Template, Clone)]
#[template(path = "upload_link.html")]
pub struct UploadLinkTemplate {
    pub app_name: String,
    pub upload_url: String,
    pub upload_id: String
}

#[derive(Template, Clone)]
#[template(path = "about.html")]
pub struct AboutTemplate {
    pub app_name: Arc<String>
}

impl From<&TranspoConfig> for AboutTemplate {
    fn from(config: &TranspoConfig) -> Self {
        Self {
            app_name: Arc::new(config.app_name.clone())
        }
    }
}

#[derive(Template, Clone)]
#[template(path = "download.html")]
pub struct DownloadTemplate {
    pub file_id: String,
    pub app_name: String
}

/*
impl UploadLinkTemplate {
    pub fn new(app_name: String, upload_url: String, upload_id: String) -> Self {
        Self {
            app_name: app_name,
            upload_url: upload_url,
            upload_id: upload_id
        }
    }
}
*/
