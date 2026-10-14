use crate::index::DeleteEvent;

/// DELETE access right.
const DELETE: u32 = 0x10000;

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Text of `<Data Name='name'>...</Data>` (either quote style).
fn data(xml: &str, name: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let open = format!("<Data Name={q}{name}{q}>");
        if let Some(i) = xml.find(&open) {
            let rest = &xml[i + open.len()..];
            return Some(unescape(&rest[..rest.find("</Data>")?]));
        }
    }
    None
}

/// Value of the `SystemTime` attribute of `<TimeCreated .../>`.
fn time_created(xml: &str) -> Option<String> {
    let rest = &xml[xml.find("<TimeCreated")?..];
    let rest = &rest[rest.find("SystemTime=")? + "SystemTime=".len()..];
    let q = rest.chars().next()?;
    let rest = &rest[1..];
    Some(rest[..rest.find(q)?].to_string())
}

/// A file deletion from an event 4663 rendered as XML; None for any other event.
pub fn parse_4663(xml: &str) -> Option<DeleteEvent> {
    if !xml.contains("<EventID>4663</EventID>") || data(xml, "ObjectType")? != "File" {
        return None;
    }
    let mask = data(xml, "AccessMask")?;
    let mask = u32::from_str_radix(mask.trim().trim_start_matches("0x"), 16).ok()?;
    if mask & DELETE == 0 {
        return None;
    }
    let utc = chrono::DateTime::parse_from_rfc3339(&time_created(xml)?).ok()?.timestamp();
    Some(DeleteEvent {
        path: data(xml, "ObjectName")?,
        user: format!("{}\\{}", data(xml, "SubjectDomainName")?, data(xml, "SubjectUserName")?),
        sid: data(xml, "SubjectUserSid").unwrap_or_default(),
        utc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of EvtRender(EvtRenderEventXml) output for 4663 (single quotes).
    fn sample(object_type: &str, mask: &str, name: &str) -> String {
        format!(
            "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
             <Provider Name='Microsoft-Windows-Security-Auditing' Guid='{{54849625-5478-4994-a5ba-3e3b0328c30d}}'/>\
             <EventID>4663</EventID><Version>1</Version><Level>0</Level><Task>12800</Task>\
             <TimeCreated SystemTime='2026-10-14T19:44:07.1234567Z'/><EventRecordID>29778</EventRecordID>\
             <Channel>Security</Channel><Computer>SRV-ARQUIVOS.auster.local</Computer></System>\
             <EventData><Data Name='SubjectUserSid'>S-1-5-21-1-2-3-1104</Data>\
             <Data Name='SubjectUserName'>joao.silva</Data><Data Name='SubjectDomainName'>AUSTER2</Data>\
             <Data Name='SubjectLogonId'>0x4367b</Data><Data Name='ObjectServer'>Security</Data>\
             <Data Name='ObjectType'>{object_type}</Data><Data Name='ObjectName'>{name}</Data>\
             <Data Name='HandleId'>0x1a4</Data><Data Name='AccessList'>%%1537</Data>\
             <Data Name='AccessMask'>{mask}</Data><Data Name='ProcessId'>0x4</Data>\
             <Data Name='ProcessName'></Data><Data Name='ResourceAttributes'>S:AI</Data></EventData></Event>"
        )
    }

    #[test]
    fn parses_user_path_and_time_and_unescapes() {
        let e = parse_4663(&sample("File", "0x10000", "D:\\Dados\\R&amp;D\\a.txt")).unwrap();
        assert_eq!(e.path, "D:\\Dados\\R&D\\a.txt");
        assert_eq!(e.user, "AUSTER2\\joao.silva");
        assert_eq!(e.sid, "S-1-5-21-1-2-3-1104");
        assert_eq!(e.utc, 1792007047);
    }

    #[test]
    fn accepts_double_quotes_and_combined_masks() {
        let xml = sample("File", "0x10080", "D:\\a.txt").replace('\'', "\"");
        assert!(parse_4663(&xml).is_some());
    }

    #[test]
    fn rejects_other_objects_masks_and_events() {
        assert!(parse_4663(&sample("Key", "0x10000", "\\REGISTRY\\x")).is_none());
        assert!(parse_4663(&sample("File", "0x2", "D:\\a.txt")).is_none());
        assert!(parse_4663(&sample("File", "0x10000", "D:\\a.txt").replace("4663", "4660")).is_none());
        assert!(parse_4663("<Event/>").is_none());
    }
}
