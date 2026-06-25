#!/bin/zsh
# 组装 SaySo.app（菜单栏应用，双击即用，不需要终端）并装到 /Applications。
# 用固定的自签名证书签名 → 每次重新打包身份不变 → macOS 不会重置已授的权限。
# 用法：scripts/make_app.sh
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
APP="dist/SaySo.app"
IDENTITY="SaySo Self Signed"

# 1) 确保签名证书存在（首次自动创建，仅本机 login keychain，无需 sudo）
HASH="$(security find-certificate -c "$IDENTITY" -Z 2>/dev/null | awk '/SHA-1/{print $3; exit}')"
if [ -z "$HASH" ]; then
  echo "▸ 首次：创建自签名代码签名证书「$IDENTITY」…"
  TMP="$(mktemp -d)"
  cat > "$TMP/cfg" <<EOF
[req]
distinguished_name=dn
x509_extensions=v3
prompt=no
[dn]
CN=$IDENTITY
[v3]
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,codeSigning
basicConstraints=critical,CA:false
EOF
  openssl req -x509 -newkey rsa:2048 -nodes -keyout "$TMP/k.pem" -out "$TMP/c.pem" -days 3650 -config "$TMP/cfg" 2>/dev/null
  openssl pkcs12 -export -inkey "$TMP/k.pem" -in "$TMP/c.pem" -out "$TMP/c.p12" \
    -passout pass:sayso -name "$IDENTITY" -legacy -macalg sha1 2>/dev/null
  security import "$TMP/c.p12" -k ~/Library/Keychains/login.keychain-db -P sayso -A >/dev/null 2>&1
  rm -rf "$TMP"
  HASH="$(security find-certificate -c "$IDENTITY" -Z 2>/dev/null | awk '/SHA-1/{print $3; exit}')"
fi

# 显式按本机架构构建（覆盖仓库根那份强制 Windows 的 cargo 配置；arm64/Intel 都适用）
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
echo "▸ 构建 release（$TRIPLE）…"
cargo build --release --bin sayso --target "$TRIPLE" >/dev/null

echo "▸ 组装 $APP …"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp packaging/Info.plist "$APP/Contents/Info.plist"
cp packaging/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp "target/$TRIPLE/release/sayso" "$APP/Contents/MacOS/sayso"
printf 'APPL????' > "$APP/Contents/PkgInfo"

if [ -n "$HASH" ]; then
  echo "▸ 用固定证书签名（$HASH）→ 权限授权可跨版本保留"
  codesign --force --deep --sign "$HASH" "$APP" >/dev/null 2>&1
else
  echo "▸ 证书不可用，退回 ad-hoc 签名（每次重打包需重授权）"
  codesign --force --deep --sign - "$APP" >/dev/null 2>&1
fi

echo "▸ 安装到 /Applications …"
rm -rf /Applications/SaySo.app
cp -R "$APP" /Applications/SaySo.app

echo "✓ 完成：/Applications/SaySo.app（双击启动）"
