import assert from "node:assert/strict";
import test from "node:test";

import { filterTodos, remaining, toggle } from "./todos.ts";

const todos = [
  { id: "1", text: "Buy milk", completed: false },
  { id: "2", text: "Walk dog", completed: true },
  { id: "3", text: "Write tests", completed: false },
];

test("toggle flips only the matching todo", () => {
  const next = toggle(todos, "1");
  assert.equal(next[0].completed, true);
  assert.equal(next[1].completed, true);
  assert.equal(next[2].completed, false);
});

test("active keeps only incomplete todos", () => {
  const active = filterTodos(todos, "active");
  assert.deepEqual(
    active.map((todo) => todo.id),
    ["1", "3"],
  );
});

test("completed keeps only completed todos", () => {
  const completed = filterTodos(todos, "completed");
  assert.deepEqual(
    completed.map((todo) => todo.id),
    ["2"],
  );
});

test("all keeps every todo", () => {
  assert.equal(filterTodos(todos, "all").length, 3);
});

test("remaining counts incomplete todos", () => {
  assert.equal(remaining(todos), 2);
});
