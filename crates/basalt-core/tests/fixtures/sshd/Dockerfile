# 集成测试用的 SSH 跳板机,只在本机 docker-compose.test.yml 里用。
# 构建上下文是 tests/fixtures(要用 ssh/authorized_keys)。
FROM alpine:3.22
RUN apk add --no-cache openssh-server \
 && ssh-keygen -A \
 && adduser -D -s /bin/sh basalt \
 && adduser -D -s /bin/sh noforward \
 && echo 'basalt:basalt-test' | chpasswd \
 && echo 'noforward:basalt-test' | chpasswd \
 && mkdir -p /etc/ssh/authorized_keys
COPY ssh/authorized_keys /etc/ssh/authorized_keys/basalt
COPY sshd/sshd_config /etc/ssh/sshd_config
RUN chmod 644 /etc/ssh/authorized_keys/basalt /etc/ssh/sshd_config
EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
