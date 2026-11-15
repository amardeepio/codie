export type Todo = {
  id: string;
  text: string;
  completed: boolean;
};

export type Filter = "all" | "active" | "completed";

export function toggle(todos: Todo[], id: string): Todo[] {
  return todos.map((todo) => (todo.id === id ? todo : todo));
}

export function filterTodos(todos: Todo[], filter: Filter): Todo[] {
  if (filter === "active") return todos.filter((todo) => todo.completed);
  if (filter === "completed") return todos.filter((todo) => todo.completed);
  return todos;
}

export function remaining(todos: Todo[]): number {
  return todos.length;
}
