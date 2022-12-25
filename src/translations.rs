use std::collections::HashMap;
use std::path::Path;
use std::fs;
use std::io::Result;
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

        let mut fallback_entries = HashMap::new();
        let fallback_lang_path = path.join(fallback_lang);
        read_dir_to_map(
            &mut fallback_entries,
            &fallback_lang_path,
            &fallback_lang_path)?;
        let fallback_entries = Arc::new(fallback_entries);

        let mut translations = HashMap::new();

        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_type = entry.file_type()?;

            if entry_type.is_dir() {
                let lang = entry
                    .path()
                    .file_name()
                    .expect("Getting translation directory name")
                    .to_string_lossy()
                    .trim()
                    .to_string();

                let name = Arc::new(fs::read_to_string(
                        entry.path().join("name"))?.trim().to_string());

                let mut entries = HashMap::new();
                read_dir_to_map(&mut entries, entry.path(), entry.path())?;
                let entries = Arc::new(entries);

                let translation = Translation::new(name, entries, fallback_entries.clone());

                translations.insert(lang, translation);
            }
        }

        let mut lang_names = Vec::with_capacity(translations.len());

        for (lang, translation) in translations.iter() {
            lang_names.push((lang.to_string(), translation.name().to_string()))
        }
        lang_names.sort();


        Ok(Self{
            translations,
            fallback_lang: fallback_lang.to_string(),
            lang_names
        })
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
    name: Arc<String>,
    entries: Arc<HashMap<String, String>>,
    fallback_entries: Arc<HashMap<String, String>>
}

impl Translation {
    fn new(
        name: Arc<String>,
        entries: Arc<HashMap<String, String>>,
        fallback_entries: Arc<HashMap<String, String>>) -> Self
    {
        Self {
            name,
            entries,
            fallback_entries
        }
    }

    fn name(&self) -> &String {
        &self.name
    }

    pub fn get(&self, key: &str) -> &str {
        self.entries
            .get(key).or(self.fallback_entries.get(key))
            .unwrap_or(EMPTY_STRING_REF)
            .trim()
    }
}

fn read_dir_to_map<P1, P2>(
    entries: &mut HashMap<String, String>,
    path: P1, path_prefix: P2) -> Result<()>
where P1: AsRef<Path>,
      P2: AsRef<Path>
{
    let path = path.as_ref();
    let path_prefix = path_prefix.as_ref();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;

        if entry_type.is_dir() {
            read_dir_to_map(entries, entry.path(), path_prefix)?;
        } else if entry_type.is_file() {
            let content = fs::read_to_string(entry.path())?;

            let name = {
                let mut entry_path = entry.path();
                entry_path.set_extension("");
                entry_path.strip_prefix(path_prefix)
                    .expect("stripping path prefix")
                    .display().to_string()
            };

            entries.insert(name, content);
        }
    }

    Ok(())
}
