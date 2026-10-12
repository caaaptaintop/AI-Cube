use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::TriggerMode;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TriggerSource {
    pub skill_md: String,
    pub openai_yaml: Option<String>,
}

impl TriggerSource {
    pub fn read(root: &Path) -> Result<Self> {
        use std::io::Read;
        let directory = crate::package_fs::Directory::open(root)?;
        let mut skill_md = String::new();
        directory.read(Path::new("SKILL.md"))?.read_to_string(&mut skill_md)?;
        let openai_yaml = match directory.read(Path::new("agents/openai.yaml")) {
            Ok(mut file) => { let mut content = String::new(); file.read_to_string(&mut content)?; Some(content) },
            Err(error) if crate::package_fs::is_not_found(&error) => None,
            Err(error) => return Err(error),
        };
        Ok(Self {
            skill_md,
            openai_yaml,
        })
    }

    pub fn render(&self, mode: TriggerMode) -> Result<Self> {
        if mode == TriggerMode::FollowSkill {
            return Ok(self.clone());
        }
        let manual = mode == TriggerMode::Manual;
        let (frontmatter, body) = split_frontmatter(&self.skill_md);
        let mut fields = match frontmatter {
            Some(frontmatter) => {
                serde_yaml::from_str::<Mapping>(frontmatter).context("技能的 YAML 配置无法修改")?
            }
            None => Mapping::new(),
        };
        fields.insert(key("disable-model-invocation"), Value::Bool(manual));
        let metadata = mapping_field(&mut fields, "metadata")?;
        metadata.insert(key("opencode/autoinvoke"), Value::Bool(!manual));
        if let Some(Value::Mapping(opencode)) = metadata.get_mut(&key("opencode")) {
            opencode.remove(&key("autoinvoke"));
        }
        let skill_md = format!("---\n{}---\n{body}", serde_yaml::to_string(&fields)?);

        let mut openai = match self.openai_yaml.as_deref() {
            Some(content) => {
                serde_yaml::from_str::<Mapping>(content).context("Codex 技能配置无法修改")?
            }
            None => Mapping::new(),
        };
        mapping_field(&mut openai, "policy")?
            .insert(key("allow_implicit_invocation"), Value::Bool(!manual));
        let openai_yaml = Some(serde_yaml::to_string(&openai)?);
        Ok(Self {
            skill_md,
            openai_yaml,
        })
    }

    pub fn write(&self, root: &Path) -> Result<()> {
        let directory = crate::package_fs::Directory::open(root)?;
        directory.write(Path::new("SKILL.md"), self.skill_md.as_bytes())?;
        if let Some(content) = &self.openai_yaml {
            directory.write(Path::new("agents/openai.yaml"), content.as_bytes())?;
        } else {
            match directory.read(Path::new("agents/openai.yaml")) {
                Ok(_) => directory.remove(Path::new("agents/openai.yaml"), &crate::update_protection::Node::File { sha256: String::new(), mode: 0 })?,
                Err(error) if crate::package_fs::is_not_found(&error) => {},
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

fn mapping_field<'a>(mapping: &'a mut Mapping, name: &str) -> Result<&'a mut Mapping> {
    let field = mapping
        .entry(key(name))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let Value::Mapping(field) = field else {
        bail!("技能的 YAML 配置无法修改");
    };
    Ok(field)
}

fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (None, content);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    if let Some(frontmatter) = rest.strip_suffix("---") {
        return (Some(frontmatter), "");
    }
    (None, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> TriggerSource {
        TriggerSource {
            skill_md: "---\nname: demo\ndescription: Demo\nmetadata:\n  custom: keep\n---\nBody\n"
                .into(),
            openai_yaml: Some("interface:\n  display_name: Demo\n".into()),
        }
    }

    #[test]
    fn trigger_modes_update_each_supported_provider_setting() {
        let manual = source().render(TriggerMode::Manual).unwrap();
        let (frontmatter, body) = split_frontmatter(&manual.skill_md);
        let frontmatter = serde_yaml::from_str::<Value>(frontmatter.unwrap()).unwrap();
        assert_eq!(frontmatter["disable-model-invocation"], Value::Bool(true));
        assert_eq!(
            frontmatter["metadata"]["opencode/autoinvoke"],
            Value::Bool(false)
        );
        assert_eq!(body, "Body\n");
        let openai = serde_yaml::from_str::<Value>(manual.openai_yaml.as_deref().unwrap()).unwrap();
        assert_eq!(
            openai["policy"]["allow_implicit_invocation"],
            Value::Bool(false)
        );
        assert_eq!(openai["interface"]["display_name"], "Demo");

        let automatic = source().render(TriggerMode::Automatic).unwrap();
        let (frontmatter, _) = split_frontmatter(&automatic.skill_md);
        let frontmatter = serde_yaml::from_str::<Value>(frontmatter.unwrap()).unwrap();
        assert_eq!(frontmatter["disable-model-invocation"], Value::Bool(false));
        assert_eq!(
            frontmatter["metadata"]["opencode/autoinvoke"],
            Value::Bool(true)
        );
        let openai =
            serde_yaml::from_str::<Value>(automatic.openai_yaml.as_deref().unwrap()).unwrap();
        assert_eq!(
            openai["policy"]["allow_implicit_invocation"],
            Value::Bool(true)
        );
    }

    #[test]
    fn following_the_skill_restores_the_original_files_exactly() {
        let source = source();
        assert_eq!(source.render(TriggerMode::FollowSkill).unwrap(), source);
    }
}
