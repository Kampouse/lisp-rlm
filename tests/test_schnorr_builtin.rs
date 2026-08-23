use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::program::run_program;

fn eval(code: &str) -> lisp_rlm_wasm::types::LispVal {
    let forms = parse_all(code).unwrap_or_else(|e| panic!("parse error: {}", e));
    let mut env = lisp_rlm_wasm::types::Env::new();
    let mut state = lisp_rlm_wasm::types::EvalState::new();
    run_program(&forms, &mut env, &mut state).unwrap_or_else(|e| panic!("eval error: {}", e))
}

#[test]
fn test_sha256_empty() {
    let r = eval("(sha256 (list))");
    assert_eq!(format!("{}", r),
        "(227 176 196 66 152 252 28 20 154 251 244 200 153 111 185 36 39 174 65 228 100 155 147 76 164 149 153 27 120 82 184 85)");
}

#[test]
fn test_sha256_abc() {
    let r = eval("(sha256 (list 97 98 99))");
    assert_eq!(format!("{}", r),
        "(186 120 22 191 143 1 207 234 65 65 64 222 93 174 34 35 176 3 97 163 150 23 122 156 180 16 255 97 242 0 21 173)");
}

#[test]
fn test_schnorr_verify_vector0() {
    let pk = "(list 249 48 138 1 146 88 195 16 73 52 79 133 248 157 82 41 181 49 200 69 131 111 153 176 134 1 241 19 188 224 54 249)";
    let sig = "(list 233 7 131 31 128 132 141 16 105 165 55 27 64 36 16 54 75 223 28 95 131 7 176 8 76 85 241 206 45 202 130 21 37 246 106 74 133 234 139 113 228 130 167 79 56 45 44 229 235 238 143 219 33 114 244 119 223 73 0 211 16 83 108 0)";
    let code = format!("(schnorr-verify {} {} (list))", pk, sig);
    let r = eval(&code);
    assert_eq!(format!("{}", r), "true");
}

#[test]
fn test_schnorr_verify_bad_sig() {
    let pk = "(list 249 48 138 1 146 88 195 16 73 52 79 133 248 157 82 41 181 49 200 69 131 111 153 176 134 1 241 19 188 224 54 249)";
    let bad = "(list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)";
    let code = format!("(schnorr-verify {} {} (list))", pk, bad);
    let r = eval(&code);
    assert_eq!(format!("{}", r), "false");
}

#[test]
fn test_schnorr_verify_vector1() {
    let pk = "(list 121 190 102 126 249 220 187 172 85 160 98 149 206 135 11 7 2 155 252 219 45 206 40 217 89 242 129 91 22 248 23 152)";
    let sig = "(list 247 48 119 237 144 190 252 5 144 148 202 124 244 3 14 71 129 249 77 173 176 81 248 224 226 180 83 197 62 114 127 232 66 83 202 78 139 177 90 239 46 88 3 63 20 229 110 144 102 107 114 35 141 25 58 27 162 181 27 87 106 150 181 152)";
    let code = format!("(schnorr-verify {} {} (list))", pk, sig);
    let r = eval(&code);
    assert_eq!(format!("{}", r), "true");
}
