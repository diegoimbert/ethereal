// The mock engine (`MockTransport`): the standalone UI (`just dev-ui`), the web build's `?mock`
// mode and the tests. Its own entry, not re-exported by `@/transport`: the app loads it only
// when it runs against it (`createDefaultTransport`, apps/web `?mock`), not at launch.
export { BUILTIN_DESCRIPTORS, builtinDescriptor, clampParam } from "./builtinDevices";
export { createDemoProject, createDemoProjects, createEmptyProject } from "./demoProject";
export { MockTransport, type MockTransportOptions } from "./MockTransport";
export { MOCK_MEGA_PARAM_COUNT, MOCK_MEGA_PLUGIN_ID } from "./plugins";
