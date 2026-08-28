use std::collections::HashMap;
use std::path::Path;
use std::fs::{self, File};
use std::io::{Result, BufReader, BufRead, Error, ErrorKind};
use std::sync::Arc;

const EMPTY_STRING: String = String::new();
const EMPTY_STRING_REF: &'static String = &EMPTY_STRING;


pub struct Translations {
    translations: HashMap<String, Translation>,
    fallback_lang: String,
    lang_names: Vec<(String, String)>
}

impl Translations {
    pub fn new<P>(path: P, fallback_lang: &str) -> Result<Self>
    where P: AsRef<Path>
    {
        let path = path.as_ref();
        let mut translations = Vec::new();
        let mut fallback = None;

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            // Translations are stored in HTML files in the top level
            // translations dir
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !file_name.ends_with(".html") {
                continue;
            }

            // The code (en, fr, etc.) is the file name without the extension
            let code = file_name[..file_name.len() - ".html".len()].to_string();

            let mut lines = BufReader::new(File::open(entry.path())?).lines();

            // The human-readable name (English, French, etc.) is given as the
            // first line of the translation file
            let name = lines.next().unwrap_or(Ok(code.to_string()))?;

            let mut key = String::new();
            let mut buf = String::new();
            let mut map = HashMap::new();
            let mut is_first_line = true;

            while let Some(line) = lines.next() {
                let line = line?;

                // Keys are given as HTML comments on a single line
                if line.starts_with("<!--") && line.ends_with("-->") {
                    let mut val = buf.clone();
                    val.shrink_to_fit();
                    map.insert(key, val);
                    buf.clear();

                    // The key is the first word in the comment
                    let line = &line["<!--".len()..line.len() - "-->".len()].trim();
                    key = line.split(' ').nth(0).unwrap_or("").to_string();
                    is_first_line = true;
                } else {
                    if is_first_line {
                        is_first_line = false;
                    } else {
                        buf.push_str(" ");
                    }
                    buf.push_str(&line);
                }
            }

            let map = Arc::new(map);
            if code == fallback_lang {
                fallback = Some(map.clone());
            }
            translations.push((code, Arc::new(name), map));
        }
        
        let fallback = match fallback {
            Some(fallback) => fallback,
            None => return Err(Error::new(ErrorKind::Other, format!("Fallback language was set to `{fallback_lang}`, but there was no `{fallback_lang}.html` file in the translations directory.")))
        };

        let mut this = Self {
            translations: HashMap::with_capacity(translations.len()),
            fallback_lang: fallback_lang.to_string(),
            lang_names: Vec::with_capacity(translations.len()),
        };

        for (code, name, map) in translations {
            let translation = Translation {
                entries: map.clone(),
                fallback_entries: fallback.clone()
            };
            this.translations.insert(code.clone(), translation);
            this.lang_names.push((code, name.to_string()));
        }

        this.lang_names.sort();

        Ok(this)
    }

    pub fn get(&self, lang: &str) -> Translation {
        self.translations.get(lang)
            .or(self.translations.get(&self.fallback_lang))
            .map(|t| t.clone())
            .expect(&format!("Getting translations for lang `{lang}`"))
    }

    pub fn names(&self) -> &[(String, String)] {
        &self.lang_names
    }
}


// The contents for a single translation
#[derive(Clone)]
pub struct Translation {
    entries: Arc<HashMap<String, String>>,
    fallback_entries: Arc<HashMap<String, String>>
}

impl Translation {
    pub fn get(&self, key: &str) -> &str {
        self.entries
            .get(key).or(self.fallback_entries.get(key))
            .unwrap_or(EMPTY_STRING_REF)
            .trim()
    }
}
