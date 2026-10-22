// 连接对话框的「SSH」「SSL」两页。表单状态在 ConnectionDialog 里,这里只负责显示和改。
import { FolderOpen } from "lucide-react";
import { getBackend, type CredentialFile } from "../../lib/api";
import { secretStoreName } from "../../lib/platform";
import type { SshAuthMethod, SslMode } from "../../lib/types";
import { Button } from "../ui/Button";
import { Checkbox, Field, SelectField } from "../ui/Field";
import { SSL_MODE_LABELS, sshSecretLabel, withSshAuth, type ConnectionForm } from "./connectionForm";

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
];

export function SshPage({ form, update, replace, errorFor }: PageProps) {
  const off = !form.sshEnabled;
  const secretLabel = sshSecretLabel(form.sshAuth);
  const keepsSaved = form.hadSavedSshSecret && !form.sshSecretTouched;
  const secretPlaceholder = keepsSaved
    ? `已保存在${secretStoreName()}`
    : form.sshAuth === "privateKey"
      ? "没有口令就留空"
      : "";
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
        onValueChange={(auth) => replace(withSshAuth(form, auth))}
      />
      <Field
        label={secretLabel}
        type="password"
        value={form.sshSecret}
        placeholder={secretPlaceholder}
        disabled={off}
        onValueChange={(sshSecret) => update({ sshSecret, sshSecretTouched: true })}
      />
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
      <div className="span-full">
        <Checkbox
          label={`${before("把", secretLabel)}保存到${secretStoreName()}`}
          checked={form.sshSaveSecret}
          disabled={off}
          onCheckedChange={(sshSaveSecret) => update({ sshSaveSecret })}
        />
      </div>
    </div>
  );
}

const SSL_MODES: readonly { value: SslMode; hint: string }[] = [
  { value: "disabled", hint: "不加密。" },
  { value: "preferred", hint: "服务器支持就加密，不校验证书。没设置过 SSL 的连接都是这样。" },
  { value: "required", hint: "必须加密，服务器不支持就连不上；不校验证书。" },
  { value: "verifyCa", hint: "加密，并校验服务器证书由所选的 CA 签发；不核对主机名。" },
  { value: "verifyIdentity", hint: "加密，校验 CA，再核对证书里的主机名和「常规」页的主机一致。" },
];

export function SslPage({ form, update, errorFor }: Omit<PageProps, "replace">) {
  const off = form.sslMode === "disabled";
  const options = SSL_MODES.map(({ value }) => {
    const label = SSL_MODE_LABELS[value];
    // 经隧道时实际连的是本机转发端口,证书里的主机名对不上。
    const blocked = value === "verifyIdentity" && form.sshEnabled;
    return { value, label: blocked ? `${label}（经 SSH 隧道时不能用）` : label, disabled: blocked };
  });
  const hint = SSL_MODES.find((m) => m.value === form.sslMode)?.hint;
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
        disabled={off}
        error={errorFor("sslCa")}
        hint="校验证书时用；不选只认常见的公共 CA，自签的证书要选签发它的 CA"
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
