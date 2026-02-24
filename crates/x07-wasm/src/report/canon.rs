use anyhow::Result;
use serde_json::Value;

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut v = value.clone();
    crate::util::canon_value_jcs(&mut v);
    let mut out = serde_json::to_vec(&v)?;
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

pub fn canonical_pretty_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut v = value.clone();
    crate::util::canon_value_jcs(&mut v);
    let mut out = serde_json::to_vec_pretty(&v)?;
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys_and_is_stable() {
        let v = json!({"b": 1, "a": {"d": 1, "c": 2}});
        let b1 = canonical_json_bytes(&v).unwrap();
        let b2 = canonical_json_bytes(&v).unwrap();
        assert_eq!(b1, b2);
        assert_eq!(
            std::str::from_utf8(&b1).unwrap(),
            "{\"a\":{\"c\":2,\"d\":1},\"b\":1}\n"
        );
    }
}
