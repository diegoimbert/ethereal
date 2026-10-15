/**
 * The release version (repo `VERSION`), or the build's own when `VITE_ETHER_VERSION` is set at
 * build time: the dev deploy sets `<VERSION>-dev.<short sha>`, so the welcome screen tells it
 * apart from a release.
 */
import VERSION_FILE from "../../../VERSION?raw";

export const APP_VERSION = (import.meta.env.VITE_ETHER_VERSION || VERSION_FILE).trim();
