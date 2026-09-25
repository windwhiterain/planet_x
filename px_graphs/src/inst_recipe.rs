pub struct InstRecipe {
    pub op_id: &'static str,
    pub decl: &'static str,
    pub type_name: &'static str,
    pub roots: &'static [&'static str],
    pub source: &'static str,
    pub body: &'static str,
}

impl InstRecipe {
    pub fn generated_body(&self) -> String {
        let mut out = String::with_capacity(self.body.len());
        let bytes = self.body.as_bytes();
        let needle = b"ARG";
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at..].starts_with(needle) {
                let before = at.checked_sub(1).map(|index| bytes[index]);
                let after = bytes.get(at + needle.len()).copied();
                let left = before.is_none_or(|ch| !is_word_byte(ch));
                let right = after.is_none_or(|ch| !is_word_byte(ch));
                if left && right {
                    out.push('&');
                    out.push_str(self.type_name);
                    at += needle.len();
                    continue;
                }
            }
            let ch = self.body[at..].chars().next().unwrap_or(' ');
            out.push(ch);
            at += ch.len_utf8();
        }
        out
    }
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub const INSTANCES: &[InstRecipe] = &[
    InstRecipe {
        op_id: "cloud.coarse/band",
        decl: "CloudCoarse",
        type_name: "Band",
        roots: &["px_volume_alg"],
        source: "art/inst/band.rs",
        body: "px_volume_alg::coarse_with(p, i.coverage.value(), ARG)",
    },
    InstRecipe {
        op_id: "field.remap/waves",
        decl: "FieldRemap",
        type_name: "Waves",
        roots: &["px_field_alg"],
        source: "art/inst/waves.rs",
        body: "px_field_alg::remap_with(&px_field_alg::identity(), p, 1.0, i.input.value(), ARG)",
    },
    InstRecipe {
        op_id: "field.remap/latbands",
        decl: "FieldRemap",
        type_name: "LatBands",
        roots: &["px_field_alg"],
        source: "art/inst/latbands.rs",
        body: "px_field_alg::remap_with(&px_field_alg::identity(), p, 1.0, i.input.value(), ARG)",
    },
];
