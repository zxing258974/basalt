// 第一次连接某台 SSH 服务器:把它的主机密钥指纹给用户核对,确认了才记下、继续连接。
import { FingerprintPattern } from "lucide-react";
import type { HostKey } from "../../lib/types";
import { useUi } from "../../stores/ui";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

/** ssh-ed25519 → ED25519,ecdsa-sha2-nistp256 → ECDSA,和 ssh-keygen -l 显示的一样。 */
export function algorithmLabel(algorithm: string): string {
  if (algorithm.startsWith("ecdsa")) return "ECDSA";
  if (algorithm.startsWith("rsa") || algorithm === "ssh-rsa") return "RSA";
  return algorithm.replace(/^ssh-/, "").toUpperCase();
}

/** 在服务器上查指纹的命令。 */
function keygenCommand(hostKey: HostKey): string {
  const file = algorithmLabel(hostKey.algorithm).toLowerCase();
  return `ssh-keygen -lf /etc/ssh/ssh_host_${file}_key.pub`;
}

function address(hostKey: HostKey): string {
  return hostKey.host.includes(":") ? `[${hostKey.host}]:${hostKey.port}` : `${hostKey.host}:${hostKey.port}`;
}

export function HostKeyPrompt() {
  const prompt = useUi((s) => s.hostKeyPrompt);
  const answer = useUi((s) => s.answerHostKey);
  if (!prompt) return null;
  const { hostKey } = prompt;
  return (
    <Dialog
      open
      title="核对 SSH 服务器"
      subtitle={<span className="mono">{address(hostKey)}</span>}
      width={460}
      onClose={() => answer(false)}
    >
      <div className="stack">
        <p className="dialog-text">
          第一次连接这台 SSH 服务器。请向服务器管理员核对下面的主机密钥指纹，或者在服务器上运行
          <code className="host-key-command selectable">{keygenCommand(hostKey)}</code>查看。
        </p>
        <div className="host-key-card">
          <FingerprintPattern aria-hidden />
          <span className="host-key-algorithm">{algorithmLabel(hostKey.algorithm)}</span>
          <span className="host-key-fingerprint mono selectable" aria-label="指纹">
            {hostKey.fingerprint}
          </span>
        </div>
        <p className="dialog-text host-key-note">
          一致才信任。信任后 Basalt 会记住它，以后指纹变了会拒绝连接；不一致说明连到的可能不是这台服务器。
        </p>
        <footer className="connection-form-footer">
          <span className="footer-spacer" />
          <Button variant="ghost" onClick={() => answer(false)}>
            取消
          </Button>
          <Button variant="primary" onClick={() => answer(true)}>
            信任并连接
          </Button>
        </footer>
      </div>
    </Dialog>
  );
}
