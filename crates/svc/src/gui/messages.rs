//! Plain pt-BR text for the errors the window shows (toasts).
use crate::config::ConfigError;
use crate::restore::RestoreError;
use crate::retention::RetentionError;

/// Drop the code tails Windows/Rust append ("(os error 5)", "(0x80070005)")
/// and the final period, keeping the readable sentence.
pub fn clean(s: &str) -> String {
    let mut t = s.trim_end();
    while let Some(i) = t.strip_suffix(')').and_then(|r| r.rfind(" (")) {
        let inner = &t[i + 2..t.len() - 1];
        if !(inner.starts_with("os error") || inner.starts_with("0x") || inner.starts_with("run as")) {
            break;
        }
        t = t[..i].trim_end();
    }
    t.trim_end_matches(['.', ' ']).to_string()
}

fn io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "sem permissão de acesso".into(),
        std::io::ErrorKind::NotFound => "arquivo ou pasta não encontrado".into(),
        std::io::ErrorKind::AlreadyExists => "o arquivo já existe".into(),
        _ => clean(&e.to_string()),
    }
}

/// Undo the `{:?}` quoting config.rs uses for paths and masks.
fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').replace("\\\\", "\\").replace("\\\"", "\"")
}

pub fn config(e: &ConfigError) -> String {
    match e {
        ConfigError::Io(e) => format!("Não foi possível acessar a configuração: {}.", io(e)),
        ConfigError::Parse(_) => {
            "O arquivo de configuração tem um erro de formato; corrija os campos e salve de novo.".into()
        }
        ConfigError::Write(_) => "Não foi possível montar o arquivo de configuração.".into(),
        ConfigError::Invalid(m) => invalid(m),
    }
}

/// `ConfigError::Invalid` texts from config.rs, in plain Portuguese.
fn invalid(m: &str) -> String {
    // normalize_drive_path: `{what} "{path}": {why}`
    if let Some((what, rest)) = m.split_once(" \"") {
        let field = match what {
            "recycle_dir" => Some("A pasta da lixeira"),
            "db_path" => Some("O arquivo do índice"),
            "folder" => Some("A pasta protegida"),
            _ => None,
        };
        if let (Some(field), Some((path, why))) = (field, rest.split_once("\": ")) {
            let path = unquote(path);
            let why = if why.starts_with("UNC") {
                r"não pode ser um caminho de rede; use um caminho local, como D:\Lixeira"
            } else if why.starts_with("must be a drive-absolute") {
                r"precisa ser um caminho completo com a letra do disco, como D:\Lixeira"
            } else if why.starts_with("must not be a drive root") {
                "não pode ser a raiz de um disco"
            } else if why.starts_with("must not contain") {
                "tem partes vazias, \".\" ou \"..\""
            } else {
                return format!("Configuração inválida: {m}.");
            };
            return format!("{field} ({path}) {why}.");
        }
    }
    if let Some(rest) = m.strip_prefix("folder ") {
        if let Some(p) = rest.strip_suffix(": include must not be empty") {
            return format!("A pasta {p} precisa de pelo menos uma máscara em Incluir (por exemplo *.*).");
        }
        if let Some((p, mask)) = rest.split_once(": invalid mask ") {
            let mask = unquote(mask.rsplit_once(" (empty").map_or(mask, |(m, _)| m));
            return format!("A pasta {p} tem uma máscara inválida ({mask}): use só nomes de arquivo, como *.pdf.");
        }
    }
    if let Some((a, b)) = m.strip_prefix("folders ").and_then(|r| r.split_once(" overlap")).and_then(|(p, _)| p.split_once(" and ")) {
        return format!("As pastas {a} e {b} se sobrepõem (uma está dentro da outra); proteja só uma delas.");
    }
    if let Some(dir) = m.strip_prefix("recycle_dir ").and_then(|r| r.split_once(" is not empty")).map(|(d, _)| d) {
        return format!("A pasta da lixeira {dir} já tem arquivos e não foi criada pelo recycle-net; escolha uma pasta vazia.");
    }
    match m {
        "max_bin_bytes must be > 0" => "O limite da lixeira deve ser maior que zero.".into(),
        "max_age_days must be > 0 when set" => {
            "Os dias de retenção devem ser maiores que zero; deixe vazio para não ter limite.".into()
        }
        "db_path must not be inside recycle_dir" => "O arquivo do índice não pode ficar dentro da pasta da lixeira.".into(),
        _ => format!("Configuração inválida: {m}."),
    }
}

/// Errors from starting/stopping/querying services through the SCM.
pub fn service(raw: &str) -> String {
    let low = raw.to_lowercase();
    if low.contains("0x80070005") || low.contains("acesso negado") || low.contains("access is denied") {
        "Sem permissão para controlar o serviço; abra o recycle-net como administrador.".into()
    } else if low.contains("is not installed") || low.contains("0x80070424") {
        "O serviço não está instalado.".into()
    } else if low.starts_with("timed out") {
        "O serviço demorou demais para parar; tente de novo em instantes.".into()
    } else if low.contains("0x8007041d") {
        r"O serviço não respondeu ao iniciar; veja o log em C:\ProgramData\recycle-net\recycle-net.log.".into()
    } else if low.contains("0x80070422") {
        "O serviço está desativado no Windows; ative-o em services.msc.".into()
    } else {
        let detail = raw.split_once(": ").map_or(raw, |(_, d)| d);
        format!("O serviço do Windows recusou a operação: {}.", clean(detail))
    }
}

/// `path` is the original path of the file being restored.
pub fn restore(e: &RestoreError, path: &str) -> String {
    match e {
        RestoreError::Exists => {
            format!("{path} já existe no local original; use \"Restaurar em...\" para salvar em outra pasta.")
        }
        RestoreError::AlreadyRestored => format!("{path} já tinha sido restaurado."),
        RestoreError::NotFound => format!("{path} não está mais na lixeira."),
        RestoreError::Io(e) => format!("{path}: {}.", io(e)),
        RestoreError::Db(_) => format!("{path}: não foi possível atualizar o índice."),
        RestoreError::BadOriginalPath(_) => format!("{path}: o caminho original é inválido."),
    }
}

pub fn purge(e: &RetentionError, path: &str) -> String {
    match e {
        RetentionError::Io(e) => format!("{path}: não foi possível excluir a cópia ({}).", io(e)),
        RetentionError::Db(_) => format!("{path}: não foi possível atualizar o índice."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, FolderRule};

    fn invalid_of(c: &Config) -> String {
        config(&c.validate().unwrap_err())
    }

    fn base() -> Config {
        Config { recycle_dir: r"E:\Lixeira".into(), folders: vec![FolderRule::with_defaults(r"D:\Dados".into())], ..Config::default() }
    }

    #[test]
    fn config_validation_errors_read_as_plain_portuguese() {
        let mut c = base();
        c.folders.push(FolderRule::with_defaults(r"D:\Dados\Sub".into()));
        assert_eq!(
            invalid_of(&c),
            r"As pastas D:\Dados e D:\Dados\Sub se sobrepõem (uma está dentro da outra); proteja só uma delas."
        );

        let mut c = base();
        c.folders[0].include.clear();
        assert_eq!(invalid_of(&c), r"A pasta D:\Dados precisa de pelo menos uma máscara em Incluir (por exemplo *.*).");

        let mut c = base();
        c.folders[0].exclude = vec![r"sub\*.tmp".into()];
        assert_eq!(
            invalid_of(&c),
            r"A pasta D:\Dados tem uma máscara inválida (sub\*.tmp): use só nomes de arquivo, como *.pdf."
        );

        let mut c = base();
        c.recycle_dir = r"\\srv\share\bin".into();
        assert_eq!(
            config(&c.normalize().unwrap_err()),
            r"A pasta da lixeira (\\srv\share\bin) não pode ser um caminho de rede; use um caminho local, como D:\Lixeira."
        );

        let mut c = base();
        c.db_path = r"index.db".into();
        assert_eq!(
            config(&c.normalize().unwrap_err()),
            r"O arquivo do índice (index.db) precisa ser um caminho completo com a letra do disco, como D:\Lixeira."
        );

        let mut c = base();
        c.recycle_dir = r"E:\".into();
        assert_eq!(config(&c.normalize().unwrap_err()), r"A pasta da lixeira (E:\) não pode ser a raiz de um disco.");

        let mut c = base();
        c.db_path = r"E:\Lixeira\index.db".into();
        assert_eq!(invalid_of(&c), "O arquivo do índice não pode ficar dentro da pasta da lixeira.");

        let mut c = base();
        c.max_bin_bytes = 0;
        assert_eq!(invalid_of(&c), "O limite da lixeira deve ser maior que zero.");

        let mut c = base();
        c.max_age_days = Some(0);
        assert_eq!(invalid_of(&c), "Os dias de retenção devem ser maiores que zero; deixe vazio para não ter limite.");
    }

    #[test]
    fn populated_unmarked_recycle_dir_is_explained() {
        let dir = std::env::temp_dir().join(format!("recycle-net-msg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("other.txt"), b"x").unwrap();
        let e = crate::config::ensure_dedicated_dir(&dir).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            config(&e),
            format!(
                "A pasta da lixeira {} já tem arquivos e não foi criada pelo recycle-net; escolha uma pasta vazia.",
                dir.display()
            )
        );
    }

    #[test]
    fn unknown_config_error_keeps_the_detail() {
        let e = crate::config::ConfigError::Invalid("something new".into());
        assert_eq!(config(&e), "Configuração inválida: something new.");
    }

    #[test]
    fn os_noise_is_stripped_and_service_errors_are_mapped() {
        assert_eq!(clean("Acesso negado. (os error 5)"), "Acesso negado");
        assert_eq!(clean("Acesso negado. (0x80070005)"), "Acesso negado");
        assert_eq!(
            service("open SCM: Acesso negado. (0x80070005) (run as Administrator)"),
            "Sem permissão para controlar o serviço; abra o recycle-net como administrador."
        );
        assert_eq!(service("recycle-net is not installed"), "O serviço não está instalado.");
        assert_eq!(
            service("timed out waiting for recycle-net to stop"),
            "O serviço demorou demais para parar; tente de novo em instantes."
        );
        assert_eq!(service("start recycle-net: Falhou. (0x8007041D)"), "O serviço não respondeu ao iniciar; veja o log em C:\\ProgramData\\recycle-net\\recycle-net.log.");
        assert_eq!(service("start x: Algo estranho. (0x80070001)"), "O serviço do Windows recusou a operação: Algo estranho.");
    }

    #[test]
    fn restore_errors_name_the_file() {
        use crate::restore::RestoreError;
        assert_eq!(
            restore(&RestoreError::Exists, r"D:\Dados\a.txt"),
            r#"D:\Dados\a.txt já existe no local original; use "Restaurar em..." para salvar em outra pasta."#
        );
        assert_eq!(restore(&RestoreError::AlreadyRestored, "x"), "x já tinha sido restaurado.");
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(restore(&RestoreError::Io(io), "x"), "x: sem permissão de acesso.");
    }
}
