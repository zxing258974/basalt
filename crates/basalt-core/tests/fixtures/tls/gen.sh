#!/bin/sh
# 重新生成集成测试用的证书(一次性测试 CA、服务器证书、客户端证书),有效期 100 年。
# 服务器证书的名字覆盖本机直连(localhost、127.0.0.1)和经 sshd 转发时用的容器名(postgres、mysql57)。
# 生成后要重建 docker-compose.test.yml 里的 mysql57 和 postgres 容器。
set -eu
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

openssl req -x509 -newkey rsa:2048 -nodes -keyout "$work/ca.key" -out ca.crt -days 36500 \
  -subj "/CN=Basalt Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"

openssl req -newkey rsa:2048 -nodes -keyout server.key -out "$work/server.csr" -subj "/CN=localhost"
printf 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,IP:127.0.0.1,DNS:postgres,DNS:mysql57\n' > "$work/server.ext"
openssl x509 -req -in "$work/server.csr" -CA ca.crt -CAkey "$work/ca.key" -CAcreateserial \
  -CAserial "$work/ca.srl" -out server.crt -days 36500 -extfile "$work/server.ext"

# 客户端证书的 CN 就是证书登录的用户名(PostgreSQL 的 basalt_cert 角色)。
openssl req -newkey rsa:2048 -nodes -keyout client.key -out "$work/client.csr" -subj "/CN=basalt_cert"
printf 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=clientAuth\n' > "$work/client.ext"
openssl x509 -req -in "$work/client.csr" -CA ca.crt -CAkey "$work/ca.key" -CAserial "$work/ca.srl" \
  -out client.crt -days 36500 -extfile "$work/client.ext"

# 一个不相干的 CA:校验 CA 时用它应该失败。
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$work/other.key" -out other-ca.crt -days 36500 \
  -subj "/CN=Unrelated CA"

chmod 644 ./*.key ./*.crt
