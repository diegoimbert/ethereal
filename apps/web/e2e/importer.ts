// The project importer (other DAWs' files) is not in every build: the open-source one has none,
// and the specs that import skip without it.
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

/** The importer crate is in this tree. */
export const HAS_IMPORTER = existsSync(resolve(repoRoot, "crates/ether-import"));
export const NO_IMPORTER = "the importer is not part of this build";
