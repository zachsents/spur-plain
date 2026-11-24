// The plain extension: Tasks', Images' and Recorder's own MCP servers (`<app> mcp`, each relaying
// to its running app and launching it in the background when a tool needs it) as one server on
// stdio. Their tools and event types carry the app's prefix (`tasks_tick`,
// `plain.tasks.data.changed`); Tasks' list widget is `ui://plain/tasks`, drawn in Switch's house
// components from Tasks' own data: a ring that ticks each item off, a line to write a new one in,
// and only the empty word when there's nothing to do.
import { randomUUID } from "node:crypto"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import path from "node:path"
import { Schema } from "effect"
import {
  A2UI_MIME_TYPE,
  BASIC_CATALOG,
  EVENTS,
  isJson,
  SUBSCRIPTION,
  surface,
  SWITCH_CATALOG,
  VERSION,
  WIDGET,
} from "switch-ext-kit"
import type { Json, Widget } from "switch-ext-kit"

import { lines, parse, spawnChild } from "./child.ts"
import type { Child } from "./child.ts"

const APPS = [
  { prefix: "tasks", name: "Tasks" },
  { prefix: "images", name: "Images" },
  { prefix: "recorder", name: "Recorder" },
] as const

const EXTENSION = "plain"
const WIDGET_URI = `ui://${EXTENSION}/tasks`
const SHOW_LIST = "tasks_show_list"

const tasksWidget = {
  name: "tasks",
  title: "Tasks",
  surfaces: ["space", "chat"],
  resource: WIDGET_URI,
  tool: SHOW_LIST,
  refresh: [`${EXTENSION}.tasks.data.changed`],
} satisfies Widget

const field = (m: Json, key: string): Json => {
  const v = m[key]
  return isJson(v) ? v : {}
}
const list = (v: unknown): readonly Json[] => (Array.isArray(v) ? v.filter(isJson) : [])
const str = (v: unknown) => (typeof v === "string" ? v : "")

const write = (m: Json) => {
  process.stdout.write(`${JSON.stringify(m)}\n`)
}
const answer = (id: unknown, result: Json) => {
  write({ jsonrpc: "2.0", id, result })
}
const fail = (id: unknown, code: number, message: string) => {
  write({ jsonrpc: "2.0", id, error: { code, message } })
}

/** Client request ids by each child's own, for subscriptions' notifications and cancelling. */
const clientIds = new Map<string, unknown>()
const childIds = new Map<unknown, { child: Child; id: number }>()

const heard = (m: Json, child: Child) => {
  const method = str(m.method)
  if (m.id !== undefined && method !== "") {
    child.send(
      method === "ping"
        ? { jsonrpc: "2.0", id: m.id, result: {} }
        : { jsonrpc: "2.0", id: m.id, error: { code: -32_601, message: `No ${method}` } },
    )
    return
  }
  if (method.startsWith("notifications/events/")) {
    const params = field(m, "params")
    const meta = field(params, "_meta")
    const id = clientIds.get(`${child.prefix}:${String(meta[SUBSCRIPTION])}`)
    if (id === undefined) {
      return
    }
    const named =
      typeof params.name === "string" ? { name: `${EXTENSION}.${child.prefix}.${params.name}` } : {}
    write({ ...m, params: { ...params, ...named, _meta: { ...meta, [SUBSCRIPTION]: id } } })
    return
  }
  if (method === "notifications/progress" || method === "notifications/message") {
    write(m)
  }
}

const dir = process.env.PLAIN_APPS ?? path.join(homedir(), "Applications")
const children = new Map<string, Child>()
const missing: string[] = []
for (const app of APPS) {
  const binary = path.join(dir, `${app.name}.app`, "Contents", "MacOS", app.prefix)
  if (existsSync(binary)) {
    children.set(app.prefix, spawnChild(app, binary, heard))
  } else {
    missing.push(app.name)
    process.stderr.write(`${app.name} isn't installed in ${dir}: bun run apps\n`)
  }
}

/** Every this often, as its manifest's `heartbeat` promises. */
const BEAT = 5 * 60_000
const HEARTBEAT = `${EXTENSION}.heartbeat`
const heartbeatEntry = {
  name: HEARTBEAT,
  description: "",
  delivery: ["push"],
  inputSchema: { type: "object", properties: {} },
  payloadSchema: Schema.toJsonSchemaDocument(
    Schema.Struct({
      entity: Schema.Literal(`sense:${EXTENSION}`),
      summary: Schema.String,
      facts: Schema.Struct({ failing: Schema.NullOr(Schema.String), apps: Schema.Finite }),
    }),
  ).schema,
  _meta: { [VERSION]: 1 },
}

/** The client's `events/stream` requests for the heartbeat, each sent every beat. */
const beatTo = new Set<unknown>()

/** How it is: its apps' servers up, what isn't and why. */
const heartbeat = () => {
  const up = [...children.values()].filter((c) => !c.exited())
  const down = [
    ...missing.map((name) => `${name} isn't installed`),
    ...[...children.values()].filter((c) => c.exited()).map((c) => `${c.name} exited`),
  ]
  return {
    entity: `sense:${EXTENSION}`,
    summary: up.map((c) => c.name).join(", ") || "no apps",
    facts: { failing: down.length === 0 ? null : down.join("; "), apps: up.length },
  }
}

const beat = () => {
  const data = heartbeat()
  for (const id of beatTo) {
    write({
      jsonrpc: "2.0",
      method: "notifications/events/event",
      params: {
        eventId: randomUUID(),
        name: HEARTBEAT,
        timestamp: new Date().toISOString(),
        data,
        _meta: { [SUBSCRIPTION]: id },
      },
    })
  }
}
const beating = setInterval(beat, BEAT)

/** A request to one child as the client's own: its answer under the client's id. */
const forward = async (child: Child, id: unknown, method: string, params: Json): Promise<Json> => {
  const key = (n: number) => `${child.prefix}:${n}`
  let sentAs = 0
  const response = await child.request(method, params, (n) => {
    sentAs = n
    clientIds.set(key(n), id)
    childIds.set(id, { child, id: n })
  })
  clientIds.delete(key(sentAs))
  childIds.delete(id)
  return { ...response, id }
}

/** Every child's result for one request: {} from one that failed. */
const everyone = async (method: string, params: Json) =>
  await Promise.all(
    [...children.values()].map(async (child) => ({
      child,
      result: field(await child.request(method, params), "result"),
    })),
  )

/** The list widget's components; Tasks' data, reshaped by `listData`, feeds them. */
const LIST_SURFACE = surface("tasks", SWITCH_CATALOG, [
  {
    id: "root",
    component: "Column",
    catalogId: BASIC_CATALOG,
    children: ["name", "to_do", "done", "add", "empty"],
  },
  {
    id: "name",
    component: "Text",
    catalogId: BASIC_CATALOG,
    text: {
      "@call": "formatString",
      catalogId: BASIC_CATALOG,
      args: { value: `# \${/name}` },
    },
  },
  {
    id: "to_do",
    component: "List",
    catalogId: BASIC_CATALOG,
    children: { componentId: "item", path: "/to_do" },
  },
  {
    id: "done",
    component: "List",
    catalogId: BASIC_CATALOG,
    children: { componentId: "item", path: "/done" },
  },
  {
    id: "item",
    component: "Check",
    label: { "@path": "label" },
    detail: { "@path": "detail" },
    done: { "@path": "done" },
    action: "tasks_tick",
    args: { "@path": "args" },
  },
  {
    id: "add",
    component: "Entry",
    placeholder: "New item",
    action: "tasks_add_item",
    args: { "@path": "/add" },
    field: "title",
  },
  {
    id: "empty",
    component: "Text",
    catalogId: BASIC_CATALOG,
    text: { "@path": "/empty" },
    variant: "caption",
  },
])

/** Tasks' widget data as the house components take it. Nothing to do shows only the empty word. */
const listData = (d: Json): Json => {
  const toDo = list(d.to_do).map((i) => ({
    label: str(i.title),
    detail: str(i.due),
    done: false,
    args: { id: str(i.id), done: true },
  }))
  const done = list(d.done).map((i) => ({
    label: str(i.title),
    detail: "",
    done: true,
    args: { id: str(i.id), done: false },
  }))
  const none = toDo.length === 0
  return {
    name: none ? "" : str(d.name),
    to_do: toDo,
    done: none ? [] : done,
    add: { list: str(d.list) },
    empty: none ? "Nothing" : "",
  }
}

const decodeMessages = (text: string): readonly unknown[] => {
  try {
    const v: unknown = JSON.parse(text)
    return Array.isArray(v) ? v : []
  } catch {
    return []
  }
}

/** `show_list`'s result: its widget in house components, and their data as the structured result. */
const shownList = (result: Json): Json => {
  let data: Json = {}
  const content = list(result.content).map((c) => {
    const r = field(c, "resource")
    if (c.type !== "resource" || r.mimeType !== A2UI_MIME_TYPE) {
      return c
    }
    for (const m of decodeMessages(str(r.text))) {
      const update = isJson(m) ? field(m, "updateDataModel") : {}
      if (isJson(update.value)) {
        data = listData(update.value)
      }
    }
    const model = { version: "1.0", updateDataModel: { surfaceId: "tasks", value: data } }
    return {
      ...c,
      resource: { ...r, uri: WIDGET_URI, text: JSON.stringify([...LIST_SURFACE, model]) },
    }
  })
  return { ...result, content, structuredContent: data }
}

/** A prefixed name's child and the name it knows: `tasks_tick` is Tasks' `tick`. */
const route = (name: string, separator: string) => {
  const cut = name.indexOf(separator)
  const child = cut === -1 ? undefined : children.get(name.slice(0, cut))
  return child ? { child, name: name.slice(cut + separator.length) } : null
}

const tools = async () => {
  const all = await everyone("tools/list", {})
  return all.flatMap(({ child, result }) =>
    list(result.tools)
      .filter((t) => {
        const seen: unknown = field(field(t, "_meta"), "ui").visibility
        return !Array.isArray(seen) || seen.includes("model")
      })
      .map((t) => {
        const name = `${child.prefix}_${str(t.name)}`
        const { _meta: _, ...rest } = t
        return {
          ...rest,
          name,
          title: `${child.name}: ${str(t.title)}`,
          ...(name === SHOW_LIST && { _meta: { ui: { resourceUri: WIDGET_URI } } }),
        }
      }),
  )
}

const events = async () => {
  const all = await everyone("events/list", {})
  return [
    heartbeatEntry,
    ...all.flatMap(({ child, result }) =>
      list(result.events).map((e) => ({
        ...e,
        name: `${EXTENSION}.${child.prefix}.${str(e.name)}`,
        _meta: { ...field(e, "_meta"), [VERSION]: 1 },
      })),
    ),
  ]
}

const INSTRUCTIONS =
  "Tasks, Images and Recorder: plain's apps on this computer, their tools named by app (tasks_, images_, recorder_). Changes show in the app at once; an app that isn't running is launched in the background. Every tool returns the app's state after the change. tasks_show_list is a list as a widget."

/** A response with its result changed, if it has one. */
const mapResult = (r: Json, f: (result: Json) => Json): Json =>
  isJson(r.result) ? { ...r, result: f(r.result) } : r

const empty = (id: unknown) => {
  answer(id, {})
}

const initialize = async (id: unknown, params: Json) => {
  const all = await everyone("initialize", params)
  const version = all.map(({ result }) => str(result.protocolVersion)).find((v) => v !== "")
  answer(id, {
    protocolVersion: version ?? str(params.protocolVersion),
    capabilities: { tools: {}, resources: {}, extensions: { [EVENTS]: { listChanged: false } } },
    serverInfo: { name: EXTENSION, version: "0.1.0" },
    instructions: INSTRUCTIONS,
  })
}

const callTool = async (id: unknown, params: Json) => {
  const name = str(params.name)
  const to = route(name, "_")
  if (!to) {
    answer(id, { content: [{ type: "text", text: `No tool ${name}` }], isError: true })
    return
  }
  const r = await forward(to.child, id, "tools/call", { ...params, name: to.name })
  write(name === SHOW_LIST ? mapResult(r, shownList) : r)
}

const readResource = async (id: unknown, params: Json) => {
  const uri = str(params.uri)
  if (uri === WIDGET_URI && children.has("tasks")) {
    answer(id, {
      contents: [{ uri, mimeType: A2UI_MIME_TYPE, text: JSON.stringify(LIST_SURFACE) }],
    })
    return
  }
  const to = route(uri, "://")
  if (!to) {
    fail(id, -32_002, `No resource ${uri}`)
    return
  }
  write(await forward(to.child, id, "resources/read", params))
}

const eventRequest = (method: string) => async (id: unknown, params: Json) => {
  if (str(params.name) === HEARTBEAT) {
    if (method === "events/stream") {
      beatTo.add(id)
      beat()
    } else {
      answer(id, { events: [] })
    }
    return
  }
  const to = route(str(params.name).replace(`${EXTENSION}.`, ""), ".")
  if (!to) {
    fail(id, -32_023, `No event type ${str(params.name)}`)
    return
  }
  const r = await forward(to.child, id, method, { ...params, name: to.name })
  write(
    mapResult(r, (result) => ({
      ...result,
      ...(Array.isArray(result.events) && {
        events: list(result.events).map((e) => ({
          ...e,
          name: `${EXTENSION}.${to.child.prefix}.${str(e.name)}`,
        })),
      }),
    })),
  )
}

const requests: Record<string, (id: unknown, params: Json) => void | Promise<void>> = {
  initialize,
  ping: empty,
  "logging/setLevel": empty,
  "resources/subscribe": empty,
  "resources/unsubscribe": empty,
  "tools/list": async (id) => {
    answer(id, { tools: await tools() })
  },
  "tools/call": callTool,
  "resources/list": (id) => {
    const widget = {
      uri: WIDGET_URI,
      name: tasksWidget.name,
      title: tasksWidget.title,
      mimeType: A2UI_MIME_TYPE,
      _meta: { [WIDGET]: tasksWidget },
    }
    answer(id, { resources: children.has("tasks") ? [widget] : [] })
  },
  "resources/templates/list": (id) => {
    answer(id, { resourceTemplates: [] })
  },
  "resources/read": readResource,
  "events/list": async (id) => {
    answer(id, { events: await events() })
  },
  "events/stream": eventRequest("events/stream"),
  "events/poll": eventRequest("events/poll"),
}

const notifications: Record<string, (m: Json) => void> = {
  "notifications/initialized": (m) => {
    for (const child of children.values()) {
      child.send(m)
    }
  },
  "notifications/cancelled": (m) => {
    const params = field(m, "params")
    beatTo.delete(params.requestId)
    const sent = childIds.get(params.requestId)
    if (sent) {
      sent.child.send({ ...m, params: { ...params, requestId: sent.id } })
    }
  },
}

const handle = async (m: Json) => {
  const method = str(m.method)
  if (m.id === undefined) {
    notifications[method]?.(m)
    return
  }
  const run = requests[method]
  if (run) {
    await run(m.id, field(m, "params"))
  } else {
    fail(m.id, -32_601, `No method ${method}`)
  }
}

await lines(Bun.stdin.stream(), (line) => {
  const m = parse(line)
  if (m) {
    void handle(m)
  }
})
clearInterval(beating)
for (const child of children.values()) {
  child.kill()
}
