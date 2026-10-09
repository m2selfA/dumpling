fn main() {
    println!("cargo:rerun-if-changed=assets/dumpling.ico.b64");

    #[cfg(windows)]
    {
        use std::fs;
        use std::path::PathBuf;

        let b64 = fs::read_to_string("assets/dumpling.ico.b64").expect("assets/dumpling.ico.b64");
        let bytes = decode_b64(&b64);
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("dumpling.ico");
        fs::write(&out, &bytes).expect("write ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(out.to_str().expect("utf8 icon path"));
        res.compile().expect("embed Windows exe icon");
    }
}

#[cfg(windows)]
fn decode_b64(input: &str) -> Vec<u8> {
    fn val(c: u8) -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let a = val(chunk[0]);
        let b = val(chunk[1]);
        out.push((a << 2) | (b >> 4));
        if chunk.len() > 2 && chunk[2] != b'=' {
            let c = val(chunk[2]);
            out.push((b << 4) | (c >> 2));
            if chunk.len() > 3 && chunk[3] != b'=' {
                out.push((c << 6) | val(chunk[3]));
            }
        }
    }
    out
}
