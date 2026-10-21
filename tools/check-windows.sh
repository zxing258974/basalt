#!/bin/sh
# 在 macOS 上检查 Windows 版能不能编译:对 x86_64-pc-windows-gnu 目标跑 clippy(含测试代码)。
# 不装 mingw / MSVC:C 依赖(ring、SQLite、Windows 资源)交给临时的假编译器,只生成空的产物。
# clippy 不链接,所以这些空产物用不上;它能查出 Rust 代码在 Windows 下的类型和 cfg 问题,
# 查不出链接、运行时的问题——那些要在 Windows 上跑(见 .github/workflows/windows.yml)。
# 第一次用先装目标:rustup target add x86_64-pc-windows-gnu
set -eu

stubs=$(mktemp -d)
trap 'rm -rf "$stubs"' EXIT

cat > "$stubs/cc" <<'STUB'
#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift ;;
    -o*) out="${1#-o}" ;;
  esac
  shift
done
[ -n "$out" ] && : > "$out"
exit 0
STUB

cat > "$stubs/ar" <<'STUB'
#!/bin/sh
for a in "$@"; do case "$a" in *.a|*.lib) printf '!<arch>\n' > "$a"; exit 0 ;; esac; done
exit 0
STUB

# embed-resource 先用 -V 探测:回一句 GNU windres 就会按 windres 的参数调用。
cat > "$stubs/windres" <<'STUB'
#!/bin/sh
case " $* " in *" -V "*) echo "GNU windres (stub for cargo check)"; exit 0 ;; esac
out=""
while [ $# -gt 0 ]; do
  [ "$1" = "--output" ] && out="$2"
  shift
done
[ -n "$out" ] && printf '!<arch>\n' > "$out"
exit 0
STUB

chmod +x "$stubs/cc" "$stubs/ar" "$stubs/windres"

CC_x86_64_pc_windows_gnu="$stubs/cc" \
CXX_x86_64_pc_windows_gnu="$stubs/cc" \
AR_x86_64_pc_windows_gnu="$stubs/ar" \
RC_x86_64_pc_windows_gnu="$stubs/windres" \
  cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
