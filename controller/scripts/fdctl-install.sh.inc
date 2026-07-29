DOWNLOAD_URL="{{download_url}}"
STAGING="/tmp/pillar-staging"
rm -rf "$STAGING"

if [ -x "{{binary_path}}" ]; then
  CURRENT=$("{{binary_path}}" version 2>/dev/null | grep -oP '\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?' | head -1 || true)
  if [ "$CURRENT" = "{{version}}" ]; then
    echo "Binary {{binary_path}} already at v{{version}}, skipping install"
  else
    echo "fdctl exists but version $CURRENT != {{version}}, will reinstall"
    CURRENT=""
  fi
fi

if [ -z "${CURRENT:-}" ]; then
  if [ -n "$DOWNLOAD_URL" ]; then
    mkdir -p "$STAGING"
    echo "Downloading fdctl from $DOWNLOAD_URL..."
    curl -sSL -o "$STAGING/fdctl" "$DOWNLOAD_URL"
    SHA="{{sha256}}"
    if [ -n "$SHA" ]; then
      echo "$SHA  $STAGING/fdctl" | sha256sum -c
      echo "SHA256 verified"
    fi
    sudo install -m 755 "$STAGING/fdctl" {{binary_path}}
  else
    echo "No download_url provided, building {{client_label}} v{{version}} from source..."
    BUILD_DIR="/tmp/firedancer-build"
    rm -rf "$BUILD_DIR"

    if ! dpkg -s build-essential clang cmake gcc make pkg-config >/dev/null 2>&1; then
      echo "Installing base build tools..."
      sudo apt-get update -qq
      sudo apt-get install -y -qq build-essential clang cmake gcc make pkg-config
    fi
    if ! dpkg -s build-essential clang cmake libudev-dev libssl-dev zlib1g-dev pkg-config libclang-dev protobuf-compiler >/dev/null 2>&1; then
      echo "Installing cargo build dependencies..."
      sudo apt-get update -qq
      sudo apt-get install -y -qq build-essential clang cmake libudev-dev libssl-dev zlib1g-dev pkg-config libclang-dev protobuf-compiler
    fi

    if ! command -v rustup >/dev/null 2>&1; then
      if [ -x "$HOME/.cargo/bin/rustup" ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
      else
        echo "Installing Rust toolchain..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        export PATH="$HOME/.cargo/bin:$PATH"
      fi
    fi

    git clone --recurse-submodules --branch v{{version}} https://github.com/firedancer-io/firedancer.git "$BUILD_DIR"
    cd "$BUILD_DIR"
    git submodule update --init --recursive

    echo "Installing dependencies (deps.sh fetch install)..."
    FD_AUTO_INSTALL_PACKAGES=1 ./deps.sh fetch install

    echo "Building fdctl (this may take 20-40 minutes)..."
    make -j fdctl

    FDCTL_BIN=$(find "$BUILD_DIR/build" -name fdctl -type f | head -1)
    if [ -z "$FDCTL_BIN" ]; then
      echo "ERROR: fdctl not found after build" >&2
      exit 1
    fi
    sudo install -m 755 "$FDCTL_BIN" {{binary_path}}
    cd /
    rm -rf "$BUILD_DIR"
    echo "Built and installed fdctl v{{version}}"
  fi
fi
echo "Binary ready at {{binary_path}}"
