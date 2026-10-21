// 模型设置对话框(mock 后端):新建、校验、测试连接、编辑时 Key 的留与清、删除、设为默认。
import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Backend } from "../../lib/api";
import { createMockBackend } from "../../lib/mockBackend";
import { findEditor, installLayoutStubs } from "../../test/editor";
import { renderApp } from "../../test/renderApp";
import { numberErrors } from "./LlmSettingsDialog";

installLayoutStubs();

const tree = () => within(screen.getByRole("tree", { name: "连接" }));
const findRow = async (name: string) =>
  (await tree().findByRole("treeitem", { name })).firstElementChild as HTMLElement;

async function openSettings(backend: Backend = createMockBackend()) {
  const app = renderApp(backend);
  const { user } = app;
  await user.dblClick(await findRow("开发"));
  await user.dblClick(await findRow("本地 MySQL"));
  await user.pointer({ keys: "[MouseRight]", target: await findRow("shop") });
  await user.click(await screen.findByRole("menuitem", { name: /新建查询/ }));
  await findEditor("查询 1");
  await user.click(screen.getByRole("button", { name: "助手" }));
  const panel = within(screen.getByRole("complementary", { name: "SQL 助手" }));
  await user.click(await panel.findByRole("button", { name: "模型设置" }));
  const dialog = within(screen.getByRole("dialog", { name: "模型设置" }));
  return { ...app, dialog, panel };
}

describe("model settings", { timeout: 15_000 }, () => {
  it("adds a Claude model after checking the fields and testing it", async () => {
    const backend = createMockBackend();
    const save = vi.spyOn(backend, "saveLlmModel");
    const { user, dialog, panel } = await openSettings(backend);
    await user.click(dialog.getByRole("button", { name: "添加模型" }));
    await user.click(dialog.getByRole("button", { name: "保存" }));
    expect(await dialog.findByText("起个名字，方便在助手里选。")).toBeInTheDocument();
    expect(dialog.getByText(/要以 http:\/\/ 或 https:\/\/ 开头/)).toBeInTheDocument();

    await user.type(dialog.getByLabelText("名称"), "公司网关 Claude");
    // 单选组支持方向键。
    dialog.getByRole("radio", { name: "OpenAI 兼容" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(dialog.getByRole("radio", { name: "Claude" })).toHaveAttribute("aria-checked", "true");
    expect(dialog.getByRole("radio", { name: "Claude" })).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(dialog.getByRole("radio", { name: "OpenAI 兼容" })).toHaveAttribute("aria-checked", "true");
    await user.click(dialog.getByRole("radio", { name: "Claude" }));
    expect(dialog.getByLabelText("Base URL")).toHaveAttribute("placeholder", "https://api.anthropic.com");
    await user.type(dialog.getByLabelText("Base URL"), "https://unreachable.example.com");
    await user.type(dialog.getByLabelText("模型名称"), "claude-sonnet-4-5");
    await user.type(dialog.getByLabelText("API Key"), "sk-ant-test");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("连不上服务器");

    await user.clear(dialog.getByLabelText("Base URL"));
    await user.type(dialog.getByLabelText("Base URL"), "https://gw.example.com/v1");
    await user.click(dialog.getByRole("button", { name: "测试连接" }));
    expect(await dialog.findByRole("status")).toHaveTextContent("连通了，420 毫秒。模型回复：OK");
    await user.click(dialog.getByRole("button", { name: "保存" }));
    expect(await dialog.findByText("已保存「公司网关 Claude」。")).toBeInTheDocument();
    expect(save.mock.calls.at(-1)![0]).toMatchObject({
      id: null,
      protocol: "anthropic",
      apiKey: "sk-ant-test",
      maxTokens: null,
      temperature: null,
    });
    const list = within(dialog.getByRole("navigation", { name: "已配置的模型" }));
    expect(list.getByRole("button", { name: /公司网关 Claude/ })).toHaveAttribute("aria-current", "true");

    await user.click(dialog.getByRole("button", { name: "设为默认模型" }));
    await waitFor(() => expect(list.getByRole("button", { name: /公司网关 Claude.*默认/ })).toBeInTheDocument());
    await user.click(dialog.getByRole("button", { name: "关闭" }));
    expect(panel.getByRole("combobox", { name: "模型" })).toHaveDisplayValue("公司网关 Claude");
  });

  it("keeps or clears a saved key when editing, and checks numbers locally", async () => {
    const backend = createMockBackend();
    const save = vi.spyOn(backend, "saveLlmModel");
    const { user, dialog } = await openSettings(backend);
    await user.click(dialog.getByRole("button", { name: /DeepSeek/ }));
    expect(dialog.getByLabelText("API Key")).toHaveAttribute("placeholder", "已存在钥匙串里，留空不改");
    expect(dialog.getByLabelText("temperature")).toHaveValue("0.2");

    await user.type(dialog.getByLabelText("最多输出 tokens"), "abc");
    await user.click(dialog.getByRole("button", { name: "保存" }));
    expect(dialog.getByText("要填正整数，不填用默认值。")).toBeInTheDocument();
    expect(save).not.toHaveBeenCalled();

    await user.clear(dialog.getByLabelText("最多输出 tokens"));
    await user.type(dialog.getByLabelText("最多输出 tokens"), "2048");
    await user.click(dialog.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save.mock.calls[0]![0]).toMatchObject({ id: "mock-deepseek", apiKey: null, maxTokens: 2048 });

    await user.click(dialog.getByRole("checkbox", { name: "清除已保存的 API Key" }));
    await user.click(dialog.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    expect(save.mock.calls[1]![0].apiKey).toBe("");
    expect(dialog.getByLabelText("API Key")).toHaveAttribute("placeholder", "本地模型（如 Ollama）可以不填");

    await user.click(dialog.getByRole("button", { name: "删除" }));
    await waitFor(() =>
      expect(within(dialog.getByRole("navigation", { name: "已配置的模型" })).queryAllByRole("listitem")).toHaveLength(
        0,
      ),
    );
  });

  it("validates numbers", () => {
    expect(numberErrors("", "")).toEqual({});
    expect(numberErrors("4096", "0.7")).toEqual({});
    expect(numberErrors("1.5", "x")).toEqual({
      maxTokens: "要填正整数，不填用默认值。",
      temperature: "要填 0 到 2 之间的数字。",
    });
    expect(numberErrors("0", "")).toHaveProperty("maxTokens");
  });
});
