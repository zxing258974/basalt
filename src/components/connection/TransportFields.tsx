// 连接对话框的「SSH」「SSL」两页。表单状态在 ConnectionDialog 里,这里只负责显示和改。
import { useQuery } from "@tanstack/react-query";
import { FolderOpen } from "lucide-react";
import { getBackend, type CredentialFile } from "../../lib/api";
import type { SshAuthMethod, SslMode } from "../../lib/types";
import { Button } from "../ui/Button";
import { Checkbox, Field, SelectField } from "../ui/Field";
import {
  SSL_MODE_LABELS,
  sshSecretLabel,
  sshUsesSecret,
  withSshAuth,
  withSshConfigHost,
  type ConnectionForm,
} from "./connectionForm";

interface PageProps {
  form: ConnectionForm;
  update(patch: Partial<ConnectionForm>): void;
  /** 换登录方式这类要整张表单一起算的改动。 */
  replace(next: ConnectionForm): void;
  errorFor(field: string): string | undefined;
}

interface FileFieldProps {
  label: string;
  value: string;
  kind: CredentialFile;
  error?: string;
  hint?: string;
  disabled: boolean;
  placeholder?: string;
  onValueChange(value: string): void;
}

/** 中文后面接拉丁字母开头的词时空一格:「选择 CA 证书」「把 SSH 密码…」。 */
const before = (prefix: string, word: string) => (/^[A-Za-z]/.test(word) ? `${prefix} ${word}` : `${prefix}${word}`);

/** 路径输入框 + 「浏览…」。 */
function FileField({ label, value, kind, error, hint, disabled, placeholder, onValueChange }: FileFieldProps) {
  const browse = async () => {
    const picked = await (await getBackend()).pickCredentialFile(kind);
    if (picked) onValueChange(picked);
  };
  return (
    <Field
      className="span-full"
      label={label}
      value={value}
      error={error}
      hint={hint}
      disabled={disabled}
      placeholder={placeholder}
      onValueChange={onValueChange}
      trailing={
        <Button
          icon={<FolderOpen />}
          disabled={disabled}
          aria-label={before("选择", label)}
          onClick={() => void browse()}
        >
          浏览…
        </Button>
      }
    />
  );
}

const INTRO =
  "先登录 SSH 服务器，再由它转发到数据库。「常规」页的主机和端口要填从 SSH 服务器那边能访问的地址，数据库就在这台服务器上时填 127.0.0.1。";

const AUTH_OPTIONS: readonly { value: SshAuthMethod; label: string }[] = [
  { value: "password", label: "密码" },
  { value: "privateKey", label: "私钥" },
  { value: "agent", label: "SSH Agent" },
];

const AGENT_HINT = "用 ssh-agent 里的私钥登录（SSH_AUTH_SOCK，或 ~/.ssh/config 里给这台主机配的 IdentityAgent）";
const JUMP_HINT =
  "跳板机用 ssh-agent 或私钥登录：~/.ssh/config 里给它配的 IdentityFile、OpenSSH 默认的私钥，或这里选的私钥";

/** 「从 ~/.ssh/config 填入」:没有这个文件(或里面没有写明名字的主机)时不显示。 */
function SshConfigPicker({ form, replace, disabled }: Pick<PageProps, "form" | "replace"> & { disabled: boolean }) {
  const hosts = useQuery({
    queryKey: ["sshConfigHosts"],
    queryFn: async () => (await getBackend()).listSshConfigHosts(),
    staleTime: 30_000,
  });
  const list = hosts.data ?? [];
  if (list.length === 0) return null;
  const options = [
    { value: "", label: "选择一台主机…" },
    ...list.map((host) => ({
      value: host.alias,
      label: host.alias === host.host ? host.alias : `${host.alias}（${host.host}）`,
    })),
  ];
  return (
    <SelectField
      className="span-full"
      label="从 ~/.ssh/config 填入"
      value=""
      options={options}
      disabled={disabled}
      hint="填好地址、端口、用户名和跳板机；配了 IdentityFile 用私钥登录，没配用 SSH Agent"
      onValueChange={(alias) => {
        const host = list.find((h) => h.alias === alias);
        if (host) replace(withSshConfigHost(form, host));
      }}
    />
  );
}

export function SshPage({ form, update, replace, errorFor }: PageProps) {
  const off = !form.sshEnabled;
  const usesSecret = sshUsesSecret(form.sshAuth);
  const secretLabel = sshSecretLabel(form.sshAuth);
  const keepsSaved = form.hadSavedSshSecret && !form.sshSecretTouched;
  const secretPlaceholder = keepsSaved ? "已保存在本机" : form.sshAuth === "privateKey" ? "没有口令就留空" : "";
  return (
    <div className="form-grid">
      <div className="span-full connection-page-intro">
        <Checkbox
          label="经 SSH 隧道连接"
          checked={form.sshEnabled}
          onCheckedChange={(sshEnabled) => update({ sshEnabled })}
        />
        <p className="field-hint">{INTRO}</p>
      </div>
      <SshConfigPicker form={form} replace={replace} disabled={off} />
      <Field
        className="span-4"
        label="SSH 主机"
        value={form.sshHost}
        disabled={off}
        error={errorFor("sshHost")}
        onValueChange={(sshHost) => update({ sshHost })}
      />
      <Field
        label="SSH 端口"
        value={form.sshPort}
        inputMode="numeric"
        disabled={off}
        error={errorFor("sshPort")}
        onValueChange={(sshPort) => update({ sshPort })}
      />
      <Field
        label="SSH 用户名"
        value={form.sshUsername}
        disabled={off}
        error={errorFor("sshUsername")}
        onValueChange={(sshUsername) => update({ sshUsername })}
      />
      <SelectField
        label="登录方式"
        value={form.sshAuth}
        options={AUTH_OPTIONS}
        disabled={off}
        hint={form.sshAuth === "agent" ? AGENT_HINT : undefined}
        onValueChange={(auth) => replace(withSshAuth(form, auth))}
      />
      {usesSecret && (
        <Field
          label={secretLabel}
          type="password"
          value={form.sshSecret}
          placeholder={secretPlaceholder}
          disabled={off}
          onValueChange={(sshSecret) => update({ sshSecret, sshSecretTouched: true })}
        />
      )}
      {form.sshAuth === "privateKey" && (
        <FileField
          label="私钥文件"
          kind="sshKey"
          value={form.sshKeyPath}
          disabled={off}
          placeholder="~/.ssh/id_ed25519"
          error={errorFor("sshPrivateKey")}
          hint="支持 OpenSSH、PKCS#8 和 PEM 格式；PuTTY 的 .ppk 要先导出成 OpenSSH 格式"
          onValueChange={(sshKeyPath) => update({ sshKeyPath })}
        />
      )}
      <Field
        className="span-full"
        label="跳板机"
        value={form.sshJumpHosts}
        disabled={off}
        placeholder="ops@bastion:22，多台用逗号；可写 ~/.ssh/config 里的别名"
        hint={JUMP_HINT}
        error={errorFor("sshJump")}
        onValueChange={(sshJumpHosts) => update({ sshJumpHosts })}
      />
      {usesSecret && (
        <div className="span-full">
          <Checkbox
            label={`${before("把", secretLabel)}保存在本机`}
            checked={form.sshSaveSecret}
            disabled={off}
            onCheckedChange={(sshSaveSecret) => update({ sshSaveSecret })}
          />
        </div>
      )}
    </div>
  );
}

const SSL_MODES: readonly { value: SslMode; hint: string }[] = [
  { value: "disabled", hint: "不加密。" },
  { value: "preferred", hint: "服务器支持就加密，不校验证书。没设置过 SSL 的连接都是这样。" },
  { value: "required", hint: "必须加密，服务器不支持就连不上；选了 CA 证书就按它校验，不选不校验证书。" },
  { value: "verifyCa", hint: "加密，并校验服务器证书由所选的 CA 签发；不核对主机名。" },
  { value: "verifyIdentity", hint: "加密，校验 CA，再核对证书里的主机名和「常规」页的主机一致。" },
];

/** CA 证书框的说明:「优先」用不到它;「必须」选了它就校验(和 libpq、mysql 客户端一样)。 */
function caHint(mode: SslMode): string {
  switch (mode) {
    case "disabled":
    case "preferred":
      return "「禁用」「优先」不校验证书，用不到 CA 证书；要校验请选「必须」或「校验 CA」";
    case "required":
      return "选了就按它校验服务器证书（和 libpq、mysql 客户端一样），不选只加密不校验";
    case "verifyCa":
    case "verifyIdentity":
      return "不选只认常见的公共 CA，自签的证书要选签发它的 CA";
  }
}

/** Redis 的 TLS 由 Basalt 自己握手:没有「优先」,经隧道也能核对主机名。 */
const REDIS_DISABLED_HINT =
  "不加密。Redis 不会在明文和 TLS 之间协商：服务器开了 TLS 就选「必须」或更严的一档，TLS 端口往往也不一样（如 6380）。";

export function SslPage({ form, update, errorFor }: Omit<PageProps, "replace">) {
  const redis = form.driver === "redis";
  const off = form.sslMode === "disabled";
  const caUnused = off || form.sslMode === "preferred";
  const options = SSL_MODES.filter(({ value }) => !(redis && value === "preferred")).map(({ value }) => {
    const label = SSL_MODE_LABELS[value];
    // 经隧道时实际连的是本机转发端口,证书里的主机名对不上(Redis 除外,见上)。
    const blocked = value === "verifyIdentity" && form.sshEnabled && !redis;
    return { value, label: blocked ? `${label}（经 SSH 隧道时不能用）` : label, disabled: blocked };
  });
  const hint = redis && off ? REDIS_DISABLED_HINT : SSL_MODES.find((m) => m.value === form.sslMode)?.hint;
  return (
    <div className="form-grid">
      <SelectField
        className="span-4"
        label="SSL 模式"
        value={form.sslMode}
        options={options}
        hint={hint}
        error={errorFor("sslMode")}
        onValueChange={(sslMode) => update({ sslMode })}
      />
      <FileField
        label="CA 证书"
        kind="certificate"
        value={form.sslCa}
        disabled={caUnused}
        error={errorFor("sslCa")}
        hint={caHint(form.sslMode)}
        onValueChange={(sslCa) => update({ sslCa })}
      />
      <FileField
        label="客户端证书"
        kind="certificate"
        value={form.sslCert}
        disabled={off}
        error={errorFor("sslCert")}
        onValueChange={(sslCert) => update({ sslCert })}
      />
      <FileField
        label="客户端私钥"
        kind="key"
        value={form.sslKey}
        disabled={off}
        error={errorFor("sslKey")}
        hint="服务器要求用证书登录时才填，证书和私钥要一起填；私钥不能有口令"
        onValueChange={(sslKey) => update({ sslKey })}
      />
    </div>
  );
}
