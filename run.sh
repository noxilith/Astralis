#!/usr/bin/env bash
# Astralis dev helper.
#
#   ./run.sh app          Launch the desktop app in dev mode (hot reload)
#   ./run.sh second       Open a second app window with its own profile (try P2P on one machine)
#   ./run.sh test         Run the core test suite (default)
#   ./run.sh build        Build release installers
#   ./run.sh ui           Run just the frontend in a browser (with a mock backend)
#   ./run.sh fetch-model  Download the in-app AI model (~90 MB, one time)
#   ./run.sh seed-test-brain [--fresh] [profile]
#                         Fill the Brain with 1,000 test memories (--fresh replaces earlier ones)
#   ./run.sh seed-test-vault [profile]
#                         Make a "Test galaxy" vault with 1,000 notes (default profile: galaxy-test;
#                         open it with `./run.sh second galaxy-test`; "main" = your own data, app closed)
#   ./run.sh check|clippy|fmt|clean
#
# Extra arguments are passed through to cargo / tauri.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="$ROOT/apps/desktop"
MODEL_DIR="$APP/src-tauri/resources/models/all-MiniLM-L6-v2"
cd "$ROOT"

# This machine has gcc but not the `x86_64-linux-gnu-gcc` name cargo asks for.
if ! command -v x86_64-linux-gnu-gcc >/dev/null 2>&1 && command -v gcc >/dev/null 2>&1; then
    export CC="${CC:-gcc}"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-gcc}"
fi
# Keep builds within memory. llama.cpp's GPU code takes several GB per file to compile, and cargo
# and llama.cpp's own build each run a job per core at the same time; on a laptop without swap
# that can freeze the whole system. Fewer jobs at once, and llama.cpp without debug info (which
# is most of that memory). Override with CARGO_BUILD_JOBS=… if you have RAM to spare.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export LLAMA_LIB_PROFILE="${LLAMA_LIB_PROFILE:-Release}"

# llama.cpp (the "Ask Astralis" model runner) is C++, built with cmake.
if ! command -v x86_64-linux-gnu-g++ >/dev/null 2>&1 && command -v g++ >/dev/null 2>&1; then
    export CXX="${CXX:-g++}"
fi
# Its Rust bindings are made by bindgen, which needs C headers like stdbool.h. Without a full
# clang install, use GCC's copies.
if ! command -v clang >/dev/null 2>&1 && command -v gcc >/dev/null 2>&1 && [[ -z "${BINDGEN_EXTRA_CLANG_ARGS:-}" ]]; then
    export BINDGEN_EXTRA_CLANG_ARGS="-I$(gcc -print-file-name=include)"
fi

# A webkit2gtk built from source into a custom prefix (default ~/.local/webkit) needs to be
# on pkg-config's path for building and on the library path for running.
WEBKIT_PREFIX="${WEBKIT_PREFIX:-$HOME/.local/webkit}"
if [[ -f "$WEBKIT_PREFIX/lib/pkgconfig/webkit2gtk-4.1.pc" ]]; then
    export PKG_CONFIG_PATH="$WEBKIT_PREFIX/lib/pkgconfig:$WEBKIT_PREFIX/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    export LD_LIBRARY_PATH="$WEBKIT_PREFIX/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export XDG_DATA_DIRS="$WEBKIT_PREFIX/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
fi

need_webkit() {
    if [[ "$(uname)" == "Linux" ]] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
        echo "The desktop app needs webkit2gtk-4.1. On Arch:" >&2
        echo "  sudo pacman -S --needed webkit2gtk-4.1 base-devel" >&2
        exit 1
    fi
}

need_npm() {
    # Node installed with nvm is only on PATH in interactive shells; find it ourselves.
    if ! command -v npx >/dev/null 2>&1; then
        local nvm_bin
        nvm_bin="$(ls -d "$HOME"/.nvm/versions/node/*/bin 2>/dev/null | sort -V | tail -1)"
        if [[ -n "$nvm_bin" ]]; then
            export PATH="$nvm_bin:$PATH"
        else
            echo "Node.js (npm/npx) not found. Install Node.js 20+ (e.g. via nvm) and try again." >&2
            exit 1
        fi
    fi
    [[ -d "$APP/node_modules" ]] || (cd "$APP" && npm install)
}

# ASTRALIS_GPU=1 builds Ask Astralis with graphics-chip support (Vulkan; Metal on macOS). It's off by
# default, and in releases: a Vulkan build won't start on a computer without Vulkan drivers.
# The Vulkan build of llama.cpp needs the
# SPIRV-Headers (header-only, from Khronos); when the system doesn't have them, fetch them once
# into .deps/ and point the build there (llama.cpp looks under $VULKAN_SDK).
need_spirv() {
    local prefix="$ROOT/.deps/spirv-headers" tag="vulkan-sdk-1.4.328.0"
    if ls /usr/share/cmake/SPIRV-Headers /usr/lib*/cmake/SPIRV-Headers /usr/local/share/cmake/SPIRV-Headers >/dev/null 2>&1; then
        return
    fi
    if [[ ! -f "$prefix/share/cmake/SPIRV-Headers/SPIRV-HeadersConfig.cmake" ]]; then
        echo "Fetching SPIRV-Headers ($tag) for the GPU build (one time)…"
        local src="$ROOT/.deps/spirv-headers-src"
        rm -rf "$src" && mkdir -p "$src"
        curl -fsSL "https://github.com/KhronosGroup/SPIRV-Headers/archive/refs/tags/$tag.tar.gz" | tar -xz -C "$src" --strip-components=1
        cmake -S "$src" -B "$src/build" -DCMAKE_INSTALL_PREFIX="$prefix" -DSPIRV_HEADERS_ENABLE_TESTS=OFF -DSPIRV_HEADERS_ENABLE_INSTALL=ON >/dev/null
        cmake --install "$src/build" >/dev/null
        rm -rf "$src"
    fi
    # Some glslc builds (e.g. shaderc 2026.x) have a newer front end than optimizer: they compile
    # recent shader features (NVIDIA coopmat2, FP4…) but then fail to optimize them ("Invalid
    # capability operand"), which breaks the build. llama.cpp finds glslc under $VULKAN_SDK/bin
    # first, so a small wrapper there retries such a shader without -O (the GPU driver optimizes
    # shaders itself when it loads them).
    local real wrapper="$prefix/bin/glslc"
    real="$(command -v glslc || true)"
    if [[ -n "$real" ]] && ! grep -q "astralis-glslc-wrapper v4" "$wrapper" 2>/dev/null; then
        mkdir -p "$prefix/bin"
        cat > "$wrapper" <<EOF
#!/usr/bin/env bash
# astralis-glslc-wrapper v4 — made by run.sh (see need_spirv there).
err="\$(mktemp)"
trap 'rm -f "\$err"' EXIT
"$real" "\$@" 2>"\$err" && { cat "\$err" >&2; exit 0; }
code=\$?
if grep -q "failed to optimize" "\$err"; then
    args=()
    for a in "\$@"; do [[ "\$a" == -O ]] || args+=("\$a"); done
    rm -f "\$err"
    exec "$real" "\${args[@]}"
fi
cat "\$err" >&2
exit \$code
EOF
        chmod +x "$wrapper"
    fi
    export VULKAN_SDK="${VULKAN_SDK:-$prefix}"
    # llama.cpp includes <spirv/unified1/spirv.hpp> as if it were installed system-wide.
    export CPLUS_INCLUDE_PATH="$prefix/include${CPLUS_INCLUDE_PATH:+:$CPLUS_INCLUDE_PATH}"
}

cmd="${1:-test}"
[ $# -gt 0 ] && shift

GPU_FEATURE=()
if [[ "${ASTRALIS_GPU:-}" == 1 ]]; then
    if [[ "$(uname)" == "Darwin" ]]; then
        GPU_FEATURE=(--features gpu-metal)
    else
        need_spirv
        GPU_FEATURE=(--features gpu-vulkan)
    fi
fi

case "$cmd" in
    app)    need_webkit; need_npm
            # The Brain's server for AI agents (Brain → Connect). Shows progress: it can take a
            # minute after updates.
            echo "Building the Brain server (astralis-mcp)…"
            cargo build -p astralis-mcp
            echo "Starting Astralis…"
            cd "$APP" && exec npx tauri dev "${GPU_FEATURE[@]}" "$@" ;;
    second) # A second window with its own profile (own notes + device identity) to try P2P on one machine.
            # Needs `./run.sh app` running (it serves the UI).
            need_webkit; ASTRALIS_PROFILE="${1:-second}" exec "$ROOT/target/debug/astralis-desktop" ;;
    build)  need_webkit; need_npm; cd "$APP" && exec npx tauri build "${GPU_FEATURE[@]}" "$@" ;;
    ui)     need_npm; cd "$APP" && exec npm run dev -- --open "$@" ;;
    test)   exec cargo test "$@" ;;
    seed-test-brain)
            DATA="${XDG_DATA_HOME:-$HOME/.local/share}/app.astralis.desktop"
            fresh=""
            if [[ "${1:-}" == "--fresh" ]]; then fresh="--fresh"; shift; fi
            profile="${1:-galaxy-test}"
            if [[ "$profile" == "main" ]]; then
                if pgrep -f "target/(debug|release)/astralis-desktop" >/dev/null; then
                    echo "Close Astralis first — it's using your main data." >&2
                    exit 1
                fi
                db="$DATA/astralis.db"
            else
                db="$DATA/profile-$profile/astralis.db"
            fi
            cargo run -q --release -p astralis-core --example seed_test_brain -- $fresh "$db"
            [[ "$profile" == "main" ]] || echo "Open it with:  ./run.sh second $profile   (needs ./run.sh app running), then Brain → Graph"
            ;;
    seed-test-vault)
            DATA="${XDG_DATA_HOME:-$HOME/.local/share}/app.astralis.desktop"
            profile="${1:-galaxy-test}"
            if [[ "$profile" == "main" ]]; then
                if pgrep -f "target/(debug|release)/astralis-desktop" >/dev/null; then
                    echo "Close Astralis first — it's using your main data." >&2
                    exit 1
                fi
                db="$DATA/astralis.db"
            else
                db="$DATA/profile-$profile/astralis.db"
            fi
            cargo run -q -p astralis-core --example seed_test_vault -- "$db"
            [[ "$profile" == "main" ]] || echo "Open it with:  ./run.sh second $profile   (needs ./run.sh app running)"
            ;;
    check)  exec cargo check --all-targets "$@" ;;
    clippy) exec cargo clippy --all-targets "$@" ;;
    fmt)    exec cargo fmt --all "$@" ;;
    clean)  exec cargo clean "$@" ;;
    fetch-model)
        # sentence-transformers/all-MiniLM-L6-v2 (Apache-2.0), from Hugging Face.
        base="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
        mkdir -p "$MODEL_DIR"
        for f in config.json tokenizer.json model.safetensors; do
            if [[ -s "$MODEL_DIR/$f" ]]; then echo "✓ $f"; continue; fi
            echo "↓ $f"
            curl -fL --progress-bar "$base/$f" -o "$MODEL_DIR/$f.part" && mv "$MODEL_DIR/$f.part" "$MODEL_DIR/$f"
        done
        echo "Model ready in $MODEL_DIR"
        ;;
    *) echo "Unknown command: $cmd (use app|second|test|build|ui|fetch-model|seed-test-vault|seed-test-brain|check|clippy|fmt|clean)" >&2; exit 1 ;;
esac
