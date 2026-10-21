import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { NumberField } from "./NumberField";

function Harness() {
  const [value, setValue] = useState(500);
  return (
    <>
      <NumberField
        label="每批"
        ariaLabel="每批的行数"
        unit="行"
        value={value}
        min={1}
        max={10000}
        onChange={setValue}
      />
      <output aria-label="值">{value}</output>
    </>
  );
}

describe("NumberField", () => {
  it("lets the field be cleared while typing and clamps when leaving", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const input = screen.getByRole("spinbutton", { name: "每批的行数" });
    await user.clear(input);
    expect(input).toHaveValue(null);
    expect(screen.getByRole("status", { name: "值" })).toHaveTextContent("500");
    await user.type(input, "0");
    expect(screen.getByRole("status", { name: "值" })).toHaveTextContent("1");
    await user.tab();
    expect(input).toHaveValue(1);
    await user.clear(input);
    await user.type(input, "99999");
    await user.tab();
    expect(input).toHaveValue(10000);
  });
});
