# TLS 测试夹具

一次性测试 CA 签发的证书,只给 `docker-compose.test.yml` 里的 MySQL 5.7 和 PostgreSQL 容器、以及 SSL 集成测试用,
不对应任何真实服务器。CA 的私钥没有留下,要换证书就运行 `gen.sh` 全部重新生成,再重建这两个容器。

| 文件 | 用途 |
|---|---|
| `ca.crt` | 测试 CA |
| `server.crt` / `server.key` | 服务器证书,名字是 localhost、127.0.0.1、postgres、mysql57 |
| `client.crt` / `client.key` | 客户端证书,CN 是 `basalt_cert`(PostgreSQL 里用证书登录的角色) |
| `other-ca.crt` | 不相干的 CA,校验 CA 时用它应该失败 |
| `pg_hba.conf` | PostgreSQL 容器的认证规则:`basalt_cert` 只能用客户端证书登录(角色由测试自己建) |
