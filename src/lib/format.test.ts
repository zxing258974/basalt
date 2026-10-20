import { describe, expect, it } from "vitest";
import { formatBytes, formatCount } from "./format";

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
