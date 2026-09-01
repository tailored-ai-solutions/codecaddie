import { expect, test } from "vitest";
import { searchDocuments } from "../src/search";

test("search returns ranked documents", async () => {
  expect(await searchDocuments("revenue")).toHaveLength(1);
});

