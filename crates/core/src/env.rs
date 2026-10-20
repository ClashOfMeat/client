use std::{collections::HashMap, path::PathBuf};

/// Home directory and environment variables used to resolve source roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// User home directory.
    pub home: PathBuf,
    /// Environment variables available to source roots.
    pub vars: HashMap<String, String>,
}

impl Env {
    /// Captures the current process home and environment variables.
    pub fn from_process() -> Self {
        Self {
            home: std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from),
            vars: std::env::vars().collect(),
        }
    }

    /// Creates an environment with a specified home and no variables.
    pub fn for_home(path: impl Into<PathBuf>) -> Self {
        Self {
            home: path.into(),
            vars: HashMap::new(),
        }
    }

    /// Resolves a root such as `~/path` or `$VAR/sub|~/default/sub`.
    pub fn resolve(&self, root: &str) -> PathBuf {
        if let Some(expression) = root.strip_prefix('$') {
            if let Some((variable_path, fallback)) = expression.split_once('|') {
                let (variable, suffix) = variable_path
                    .split_once('/')
                    .map_or((variable_path, ""), |(variable, suffix)| (variable, suffix));
                if let Some(value) = self.vars.get(variable) {
                    let mut path = PathBuf::from(value);
                    if !suffix.is_empty() {
                        path.push(suffix);
                    }
                    return path;
                }
                return self.resolve(fallback);
            }
        }
        if root == "~" {
            self.home.clone()
        } else if let Some(suffix) = root.strip_prefix("~/") {
            self.home.join(suffix)
        } else {
            PathBuf::from(root)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Env;

    #[test]
    fn resolves_plain_home_path() {
        let env = Env::for_home("/tmp/example");
        assert_eq!(
            env.resolve("~/sessions"),
            PathBuf::from("/tmp/example/sessions")
        );
    }

    #[test]
    fn uses_default_when_variable_is_unset() {
        let env = Env::for_home("/tmp/example");
        assert_eq!(
            env.resolve("$CODEX_HOME|~/default/sessions"),
            PathBuf::from("/tmp/example/default/sessions")
        );
    }

    #[test]
    fn uses_variable_when_set() {
        let mut env = Env::for_home("/tmp/example");
        env.vars.insert("CODEX_HOME".into(), "/tmp/custom".into());
        assert_eq!(
            env.resolve("$CODEX_HOME|~/default/sessions"),
            PathBuf::from("/tmp/custom")
        );
    }

    #[test]
    fn appends_suffix_to_variable() {
        let mut env = Env::for_home("/tmp/example");
        env.vars.insert("CODEX_HOME".into(), "/tmp/custom".into());
        assert_eq!(
            env.resolve("$CODEX_HOME/sessions|~/default/sessions"),
            PathBuf::from("/tmp/custom/sessions")
        );
    }
}
