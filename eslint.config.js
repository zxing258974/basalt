// @ts-check
import js from "@eslint/js";
// typescript-eslint 需要 TypeScript 的 JS 编译器接口,项目用的 TypeScript 7 没有,
// 由这个本地包带上 TypeScript 6 给它用。见 tools/typescript-eslint-ts6。
import tseslint from "@basalt/typescript-eslint-ts6";
import reactHooks from "eslint-plugin-react-hooks";
import { defineConfig, globalIgnores } from "eslint/config";
import globals from "globals";

export default defineConfig(
  // .claude/ 下是其他会话的 worktree 检出,各有各的依赖和构建产物,不属于这个工作区。
  globalIgnores([
    "dist/",
    "target/",
    "coverage/",
    "test-results/",
    "playwright-report/",
    "src-tauri/",
    "crates/",
    ".claude/",
  ]),
  {
    files: ["**/*.{js,ts,tsx}"],
    extends: [js.configs.recommended, tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      parserOptions: {
        project: ["./tsconfig.json", "./tsconfig.node.json"],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // 接口里的回调统一写成方法签名(onClose(): void),实现全是箭头函数、不用 this,
      // 解构出来调用不会丢 this,这条规则在这里只有误报。
      "@typescript-eslint/unbound-method": "off",
      // 后端错误按 Tauri 的约定以普通对象 CommandError 抛出(invoke 拒绝时给的就是它),不是 Error。
      "@typescript-eslint/only-throw-error": [
        "error",
        { allow: [{ from: "file", name: "CommandError", path: "src/lib/types.ts" }] },
      ],
      // 用解构剔除某个键(const { [id]: _removed, ...rest } = obj)是有意为之。
      "@typescript-eslint/no-unused-vars": ["error", { ignoreRestSiblings: true }],
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [reactHooks.configs.flat["recommended-latest"]],
    languageOptions: { globals: globals.browser },
  },
  {
    files: ["**/*.js"],
    extends: [tseslint.configs.disableTypeChecked],
    languageOptions: { globals: globals.node },
  },
);
