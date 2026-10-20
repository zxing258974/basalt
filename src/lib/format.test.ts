import { describe, expect, it } from "vitest";
import { formatBytes, formatCount, formatWhen } from "./format";

describe("formatBytes", () => {
  it("uses binary units with one decimal above KB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(16_384)).toBe("16 KB");
    expect(formatBytes(2_637_824)).toBe("2.5 MB");
    expect(formatBytes(3_221_225_472)).toBe("3 GB");
    expect(formatBytes(1_288_617_984)).toBe("1.2 GB");
  });

  it("renders unknown sizes as an em dash", () => {
    expect(formatBytes(null)).toBe("—");
  });
});

describe("formatCount", () => {
  it("groups thousands and marks unknown values", () => {
    expect(formatCount(412_880)).toBe("412,880");
    expect(formatCount(0)).toBe("0");
    expect(formatCount(null)).toBe("—");
  });
});

describe("formatWhen", () => {
  const now = new Date(2026, 9, 20, 15, 0, 0);
  it("shortens recent timestamps", () => {
    expect(formatWhen(new Date(2026, 9, 20, 9, 5, 7).toISOString(), now)).toBe("09:05:07");
    expect(formatWhen(new Date(2026, 8, 3, 22, 41, 0).toISOString(), now)).toBe("09-03 22:41");
    expect(formatWhen(new Date(2025, 11, 31, 8, 0, 0).toISOString(), now)).toBe("2025-12-31");
    expect(formatWhen("不是时间", now)).toBe("不是时间");
  });
});
