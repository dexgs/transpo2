use trillium_askama::Template;
use crate::config::*;

use std::cmp;


#[derive(Template, Clone, Copy)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    max_days: usize,
    max_hours: usize,
    max_minutes: usize,
    max_upload_size: usize
}

impl From<&TranspoConfig> for IndexTemplate {
    fn from(config: &TranspoConfig) -> Self {
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
            max_days: max_days,
            max_hours: max_hours,
            max_minutes: max_minutes,
            max_upload_size: config.max_upload_size_bytes
        }
    }
}
