use trillium_askama::Template;
use crate::config::*;
use crate::translations::*;

use std::cmp;


// return (max_days, max_hours, max_minutes, max_upload_size)
fn get_limits(config: &TranspoConfig) -> (usize, usize, usize, usize) {
    let max_days = cmp::max(config.max_upload_age_minutes / (24 * 60) - 1, 0);

    let max_hours = if max_days > 0 {
        23
    } else {
        cmp::max(config.max_upload_age_minutes / 60 - 1, 0)
    };

    let max_minutes = if max_hours > 1 {
        60
    } else {
        config.max_upload_age_minutes
    };

    (max_days, max_hours, max_minutes, config.max_upload_size_bytes)
}

#[derive(Template, Clone)]
#[template(path = "index.html", escape = "none")]
pub struct IndexTemplate<'a> {
    app_name: &'a String,
    selected_lang: &'a str,
    lang_names: &'a [(String, String)],
    max_days: usize,
    max_hours: usize,
    max_minutes: usize,
    max_upload_size: usize,
    t: Translation
}

impl<'a> IndexTemplate<'a> {
    pub fn new(
        config: &'a TranspoConfig,
        lang_names: &'a [(String, String)],
        selected_lang: &'a str,
        translation: Translation) -> Self
    {
        let app_name = &config.app_name;

        let (max_days, max_hours, max_minutes, max_upload_size) = get_limits(config);

        Self {
            app_name,
            selected_lang,
            lang_names,
            max_days,
            max_hours,
            max_minutes,
            max_upload_size,
            t: translation
        }
    }
}

#[derive(Template, Clone)]
#[template(path = "paste.html", escape = "none")]
pub struct PasteTemplate<'a> {
    app_name: &'a String,
    max_days: usize,
    max_hours: usize,
    max_minutes: usize,
    max_upload_size: usize,
    t: Translation
}

impl<'a> PasteTemplate<'a> {
    pub fn new(config: &'a TranspoConfig, translation: Translation) -> Self {
        let app_name = &config.app_name;
        let (max_days, max_hours, max_minutes, max_upload_size) = get_limits(config);

        Self {
            app_name,
            max_days,
            max_hours,
            max_minutes,
            max_upload_size,
            t: translation
        }
    }
}

#[derive(Template, Clone)]
#[template(path = "upload_link.html", escape = "none")]
pub struct UploadLinkTemplate {
    pub app_name: String,
    pub upload_url: String,
    pub upload_id: String,
    pub t: Translation
}

#[derive(Template, Clone)]
#[template(path = "about.html", escape = "none")]
pub struct AboutTemplate<'a> {
    pub app_name: &'a String,
    pub t: Translation
}

impl<'a> AboutTemplate<'a> {
    pub fn new(config: &'a TranspoConfig, translation: Translation) -> Self {
        Self {
            app_name: &config.app_name,
            t: translation
        }
    }
}

#[derive(Template, Clone)]
#[template(path = "download.html", escape = "none")]
pub struct DownloadTemplate<'a> {
    pub file_id: String,
    pub app_name: &'a String,
    pub has_password: bool,
    pub t: Translation
}

#[derive(Template, Clone)]
#[template(path = "error.html", escape = "none")]
pub struct ErrorTemplate<'a> {
    pub error_code: usize,
    pub app_name: &'a String,
    pub path_prefix: String,
    pub t: Translation
}
