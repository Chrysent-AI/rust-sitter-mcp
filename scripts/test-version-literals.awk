# A lightweight lexical guard, not a Rust parser. Integration tests and inline
# #[test]/#[cfg(test)] blocks are checked. Comments and string contents are tokens,
# never executable comparisons. Cargo tests remain the bump-time backstop.
function emit(token, literal,    active, direct) {
  active = integration || testdepth > 0
  if (literal) {
    direct = (assertion[depth] && operand[depth] && argument[depth] <= 2) || constructed[depth]
    if (active && token == version && (previous == "==" || previous == "!=" || direct)) report()
    operand[depth] = 0
    before = previous; previous = token; previousliteral = 1
    return
  }
  if (token == "test" && attribute) pendingtest = 1
  if (token == "#") attribute = 1
  if (token == "]") attribute = 0
  if (token == "{") {
    braces++
    if (pendingtest && !testdepth) testdepth = braces
    pendingtest = 0
  }
  if (token == "}") {
    if (braces == testdepth) testdepth = 0
    braces--
  }
  if (token == "(" || token == "[" || token == "{") {
    depth++
    # Track literal construction only at a direct assertion operand; ordinary
    # fixture calls stay excluded. Nested tokens inherit this construction scope.
    constructed[depth] = constructed[depth - 1] || (assertion[depth - 1] && operand[depth - 1] && argument[depth - 1] <= 2 && previous == "!" && (before == "concat" || before == "stringify"))
    if (previous == "!" && (before == "assert_eq" || before == "assert_ne")) {
      assertion[depth] = 1
      operand[depth] = 1
      argument[depth] = 1
    }
  }
  if (active && (token == "==" || token == "!=") && previousliteral && previous == version) report()
  if (token == "," && assertion[depth]) { operand[depth] = 1; argument[depth]++ }
  else if (token != "(" && token != "[" && token != "{" && !((token == "concat" || token == "stringify" || (token == "!" && (previous == "concat" || previous == "stringify"))) && operand[depth])) operand[depth] = 0
  if (token == ")" || token == "]" || token == "}") {
    delete assertion[depth]; delete operand[depth]; delete argument[depth]; delete constructed[depth]; depth--
  }
  before = previous; previous = token; previousliteral = literal
}
function report() {
  printf "%s:%d: hardcoded current package version \"%s\" in test comparison; use env!(\"CARGO_PKG_VERSION\").\n", FILENAME, NR, version > "/dev/stderr"
  failed = 1
}
BEGIN { integration = 0 }
FNR == 1 { integration = (FILENAME ~ /^tests\//) }
{
  line = $0 "\n"
  for (i = 1; i <= length(line); ) {
    c = substr(line, i, 1); pair = substr(line, i, 2)
    if (comment) {
      if (pair == "/*") { comment++; i += 2 }
      else if (pair == "*/") { comment--; i += 2 }
      else i++
      continue
    }
    if (string) {
      if (rawend != "") {
        if (substr(line, i, length(rawend)) == rawend) {
          emit(value, 1); i += length(rawend); string = 0; rawend = ""
        } else { value = value c; i++ }
      } else if (c == "\\") { value = value substr(line, i, 2); i += 2 }
      else if (c == "\"") { emit(value, 1); string = 0; i++ }
      else { value = value c; i++ }
      continue
    }
    if (pair == "//") break
    if (pair == "/*") { comment = 1; i += 2; continue }
    if (c ~ /[ \t\r\n]/) { i++; continue }
    rest = substr(line, i)
    if (match(rest, /^r#*"/)) {
      opener = substr(rest, 1, RLENGTH)
      rawend = "\"" substr(opener, 2, length(opener) - 2)
      string = 1; value = ""; i += RLENGTH; continue
    }
    if (c == "\"") { string = 1; value = ""; i++; continue }
    # Character literals cannot contain a version. Do not mistake lifetimes
    # for strings or skip the rest of their enclosing test function.
    if (match(rest, /^\047([^\047\\]|\\.)\047/)) { i += RLENGTH; emit("char", 0); continue }
    if (match(rest, /^[A-Za-z_][A-Za-z_0-9]*/)) {
      emit(substr(rest, 1, RLENGTH), 0); i += RLENGTH; continue
    }
    if (pair == "==" || pair == "!=") { emit(pair, 0); i += 2; continue }
    emit(c, 0); i++
  }
}
END { if (failed) exit 1 }
