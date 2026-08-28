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
fn test_sha256_hello() {
    let r = eval("(sha256 (list 104 101 108 108 111 32 119 111 114 108 100))");
    assert_eq!(format!("{}", r),
        "(185 77 39 185 147 77 62 8 165 46 82 215 218 125 171 250 196 132 239 227 122 83 128 238 144 136 247 172 226 239 205 233)");
}

#[test]
fn test_schnorr_v0() {
    let pk = "(list 249 48 138 1 146 88 195 16 73 52 79 133 248 157 82 41 181 49 200 69 131 111 153 176 134 1 241 19 188 224 54 249)";
    let sig = "(list 233 7 131 31 128 132 141 16 105 165 55 27 64 36 16 54 75 223 28 95 131 7 176 8 76 85 241 206 45 202 130 21 37 246 106 74 133 234 139 113 228 130 167 79 56 45 44 229 235 238 232 253 178 23 47 71 125 244 144 13 49 5 54 192)";
    let msg = "(list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)";
    let code = format!("(schnorr-verify {} {} {})", pk, sig, msg);
    assert_eq!(format!("{}", eval(&code)), "true");
}

#[test]
fn test_schnorr_v1() {
    let pk = "(list 223 241 215 127 42 103 28 95 54 24 55 38 219 35 65 190 88 254 174 29 162 222 206 216 67 36 15 123 80 43 166 89)";
    let sig = "(list 104 150 189 96 238 174 41 109 180 138 34 159 247 29 254 7 27 222 65 62 109 67 249 23 220 141 207 140 120 222 51 65 137 6 209 26 201 118 171 204 178 11 9 18 146 191 244 234 137 126 252 182 57 234 135 28 250 149 246 222 51 158 75 10)";
    let msg = "(list 36 63 106 136 133 163 8 211 19 25 138 46 3 112 115 68 164 9 56 34 41 159 49 208 8 46 250 152 236 78 108 137)";
    let code = format!("(schnorr-verify {} {} {})", pk, sig, msg);
    assert_eq!(format!("{}", eval(&code)), "true");
}

#[test]
fn test_schnorr_bad_sig() {
    let pk = "(list 249 48 138 1 146 88 195 16 73 52 79 133 248 157 82 41 181 49 200 69 131 111 153 176 134 1 241 19 188 224 54 249)";
    let sig = "(list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)";
    let msg = "(list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)";
    let code = format!("(schnorr-verify {} {} {})", pk, sig, msg);
    assert_eq!(format!("{}", eval(&code)), "false");
}

#[test]
fn test_schnorr_wrong_msg() {
    let pk = "(list 249 48 138 1 146 88 195 16 73 52 79 133 248 157 82 41 181 49 200 69 131 111 153 176 134 1 241 19 188 224 54 249)";
    let sig = "(list 233 7 131 31 128 132 141 16 105 165 55 27 64 36 16 54 75 223 28 95 131 7 176 8 76 85 241 206 45 202 130 21 37 246 106 74 133 234 139 113 228 130 167 79 56 45 44 229 235 238 143 219 33 114 244 119 223 73 0 211 16 83 108 0)";
    let msg = "(list 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1)";
    let code = format!("(schnorr-verify {} {} {})", pk, sig, msg);
    assert_eq!(format!("{}", eval(&code)), "false");
}

#[test]
fn test_schnorr_wrong_pk() {
    let pk = "(list 223 241 215 127 42 103 28 95 54 24 55 38 219 35 65 190 88 254 174 29 162 222 206 216 67 36 15 123 80 43 166 89)";
    let sig = "(list 233 7 131 31 128 132 141 16 105 165 55 27 64 36 16 54 75 223 28 95 131 7 176 8 76 85 241 206 45 202 130 21 37 246 106 74 133 234 139 113 228 130 167 79 56 45 44 229 235 238 143 219 33 114 244 119 223 73 0 211 16 83 108 0)";
    let msg = "(list 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0)";
    let code = format!("(schnorr-verify {} {} {})", pk, sig, msg);
    assert_eq!(format!("{}", eval(&code)), "false");
}
