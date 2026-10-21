# SSH 测试夹具

全部是为测试临时生成的一次性密钥,只用于本机测试和 `docker-compose.test.yml` 里的 sshd 容器,
不对应任何真实服务器或账号。

| 文件 | 用途 |
|---|---|
| `id_ed25519` | 客户端私钥,无口令 |
| `id_ed25519_passphrase` | 客户端私钥,口令 `basalt-key-pass` |
| `id_rsa_pem` | 客户端私钥,老式 PEM 格式的 RSA(`BEGIN RSA PRIVATE KEY`),无口令 |
| `authorized_keys` | 上面三把的公钥,sshd 容器里 `basalt` 用户的授权公钥 |
| `host_ed25519` | 单元测试里进程内 SSH 服务器的主机密钥 |

重新生成:`ssh-keygen -t ed25519 -N "" -f id_ed25519` 等,再把三把公钥拼成 `authorized_keys`。
