import { useState } from "react";
import { useConnectionActions } from "../../hooks/useConnectionActions";
import { errorMessage } from "../../lib/types";
import { useUi, type PasswordPromptState } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Checkbox, Field } from "../ui/Field";
import { sshSecretLabel } from "./connectionForm";
import { endpointSummary, needsSshPrompt, tunnelSummary } from "./driverMeta";
import { secretStoreName } from "../../lib/platform";

export function PasswordPrompt() {
  const prompt = useUi((s) => s.passwordPrompt);
  if (!prompt) return null;
  return <PasswordPromptDialog key={prompt.connection.id} prompt={prompt} />;
}

/** 要问哪几个:没存的数据库密码、数据库说密码不对;没存的 SSH 密码、后端说 SSH 那边要密码或口令。 */
function wantedFields(prompt: PasswordPromptState) {
  const { connection } = prompt;
  return {
    database: !connection.savePassword || prompt.error !== undefined,
    ssh: connection.ssh != null && (needsSshPrompt(connection) || prompt.sshError !== undefined),
  };
}

function PasswordPromptDialog({ prompt }: { prompt: PasswordPromptState }) {
  const { askPassword, notify } = useUi.getState();
  const { open, rememberPassword, rememberSshSecret } = useConnectionActions();
  const [password, setPassword] = useState("");
  const [sshSecret, setSshSecret] = useState("");
  const { connection } = prompt;
  // 后端后来又说要 SSH 口令时,数据库密码框照样留着(已经输了的不丢)。
  const [asked, setAsked] = useState(() => wantedFields(prompt));
  const wanted = wantedFields(prompt);
  if ((wanted.database && !asked.database) || (wanted.ssh && !asked.ssh)) {
    setAsked({ database: asked.database || wanted.database, ssh: asked.ssh || wanted.ssh });
  }
  // 默认记住:连接成功后写进钥匙串并把 savePassword 置为 true,用户只需输一次。
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);
  /** 认证失败由 open() 重新调用 askPassword 带回 prompt.error;其他错误记在这里。 */
  const [otherError, setOtherError] = useState<string>();
  const sshLabel = sshSecretLabel(connection.ssh?.auth ?? "password");

  const submit = async () => {
    setBusy(true);
    setOtherError(undefined);
    askPassword({ connection });
    const credentials = {
      password: asked.database ? password : undefined,
      sshSecret: asked.ssh ? sshSecret : undefined,
    };
    try {
      const opened = await open({ ...connection, isOpen: false }, credentials);
      if (!opened) return;
      askPassword(null);
      // 先确认密码能用再记住。记住失败不影响已经打开的连接,只提示。
      if (!remember) return;
      const failed = (error: unknown) => notify(`已连接，但保存密码失败：${errorMessage(error)}`);
      if (credentials.password !== undefined) await rememberPassword(connection.id, credentials.password).catch(failed);
      if (credentials.sshSecret !== undefined)
        await rememberSshSecret(connection.id, credentials.sshSecret).catch(failed);
    } catch (error) {
      setOtherError(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const tunnel = tunnelSummary(connection);
  return (
    <Dialog
      open
      title={`连接到「${connection.name}」`}
      subtitle={
        <span className="mono">
          {endpointSummary(connection)}
          {tunnel && ` · ${tunnel}`}
        </span>
      }
      width={400}
      onClose={() => askPassword(null)}
    >
      <form
        className="stack"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        {asked.ssh && (
          <Field
            label={sshLabel}
            type="password"
            value={sshSecret}
            error={prompt.sshError ?? (asked.database ? undefined : otherError)}
            onValueChange={setSshSecret}
            autoFocus
          />
        )}
        {asked.database && (
          <Field
            label="密码"
            type="password"
            value={password}
            error={otherError ?? prompt.error}
            onValueChange={setPassword}
            autoFocus={!asked.ssh}
          />
        )}
        <Checkbox
          label={`连接成功后把密码保存到${secretStoreName()}`}
          checked={remember}
          onCheckedChange={setRemember}
        />
        <footer className="connection-form-footer">
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={() => askPassword(null)}>
            取消
          </Button>
          <Button variant="primary" type="submit" busy={busy}>
            连接
          </Button>
        </footer>
      </form>
    </Dialog>
  );
}
