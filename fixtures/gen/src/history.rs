//! `history`: one repository whose `main` holds the awkward cases of a history.
//!
//! - renames at 100% (`src/util.ts` to `src/lib/util.ts`), 92% (`src/big.ts` to `src/core/big.ts`)
//!   and under 50% similarity (`src/config.ts` to `src/settings.ts`), each in its own commit;
//! - a file deleted and re-added (`src/gone.ts`), and its import closing and reopening;
//! - a merge whose second parent carries the feature commits (`src/feature.ts`);
//! - a rebase: `topic-before-rebase` keeps the original topic commits, `main` has the rebased ones;
//! - a force-push: `before-force-push` keeps the commits that `main` dropped and replaced.

use std::path::Path;

use cartograph_git::fixture::Fixture;

pub(crate) const SOURCE: &str = include_str!("history.rs");

/// The commit messages of the rename commits, for tests.
pub const RENAME_100: &str = "move util to lib (100%)";
pub const RENAME_92: &str = "move big to core (92%)";
pub const RENAME_UNDER_50: &str = "rewrite config as settings (under 50%)";

/// 25 lines of about 50 bytes. The move changes two: the import path and one value, which leaves
/// 92% similarity.
fn big(changed: bool) -> String {
    let mut s = String::from("import { util } from \"./util\";\n");
    for i in 1..25 {
        let line = if changed && i == 7 {
            format!("export const value{i:02} = util({i:02}) * 1000; // moved\n")
        } else {
            format!("export const value{i:02} = util({i:02}) + {i:04}; // kept\n")
        };
        s.push_str(&line);
    }
    s
}

pub(crate) fn generate(root: &Path) {
    let mut f = Fixture::init(&root.join("history"));
    f.set_clock(1_700_000_000);
    f.write("package.json", "{\n  \"name\": \"history\"\n}\n");
    f.write(
        "src/util.ts",
        "export function util(n: number): number {\n  return n * 2;\n}\n",
    );
    f.write("src/big.ts", &big(false));
    f.write(
        "src/config.ts",
        "export const config = {\n  port: 8080,\n  host: \"localhost\",\n  debug: false,\n};\n",
    );
    f.write("src/gone.ts", "export const gone = 1;\n");
    let app = |util: &str, big: &str, config: &str, gone: bool, feature: bool| {
        let mut s = format!(
            "import {{ util }} from \"{util}\";\nimport {{ value01 }} from \"{big}\";\nimport {{ {} }} from \"{config}\";\n",
            if config.ends_with("settings") {
                "settings"
            } else {
                "config"
            }
        );
        if gone {
            s.push_str("import { gone } from \"./gone\";\n");
        }
        if feature {
            s.push_str("import { feature } from \"./feature\";\n");
        }
        s.push_str("export const app = util(value01);\n");
        s
    };
    f.write(
        "src/app.ts",
        &app("./util", "./big", "./config", true, false),
    );
    f.commit("start");

    std::fs::create_dir_all(f.dir.join("src/lib")).expect("mkdir");
    f.git(&["mv", "src/util.ts", "src/lib/util.ts"]);
    f.write("src/big.ts", &big(false).replace("./util", "./lib/util"));
    f.write(
        "src/app.ts",
        &app("./lib/util", "./big", "./config", true, false),
    );
    f.commit(RENAME_100);

    f.remove("src/big.ts");
    f.write(
        "src/core/big.ts",
        &big(true).replace("./util", "../lib/util"),
    );
    f.write(
        "src/app.ts",
        &app("./lib/util", "./core/big", "./config", true, false),
    );
    f.commit(RENAME_92);

    f.remove("src/config.ts");
    f.write(
        "src/settings.ts",
        "// Settings now come from the environment.\nexport const settings = {\n  port: Number(process.env.PORT ?? 3000),\n  verbose: process.env.VERBOSE === \"1\",\n};\n",
    );
    f.write(
        "src/app.ts",
        &app("./lib/util", "./core/big", "./settings", true, false),
    );
    f.commit(RENAME_UNDER_50);

    f.remove("src/gone.ts");
    f.write(
        "src/app.ts",
        &app("./lib/util", "./core/big", "./settings", false, false),
    );
    f.commit("delete gone");

    f.write("src/gone.ts", "export const gone = 1;\n");
    f.write(
        "src/app.ts",
        &app("./lib/util", "./core/big", "./settings", true, false),
    );
    let base = f.commit("re-add gone");

    // A feature branch, merged with the feature as the second parent.
    f.write("src/feature.ts", "export const feature = 1;\n");
    f.commit("feature: add feature");
    f.write(
        "src/feature.ts",
        "import { util } from \"./lib/util\";\nexport const feature = util(1);\n",
    );
    let feat2 = f.commit("feature: feature uses util");
    f.git(&["branch", "feature", &feat2]);
    f.git(&["read-tree", "--reset", "-u", &base]);
    f.write("README.md", "# history\n");
    let main1 = f.commit_with_parents("docs: readme on main", &[&base]);
    f.git(&["checkout", "-q", &feat2, "--", "src/feature.ts"]);
    f.write(
        "src/app.ts",
        &app("./lib/util", "./core/big", "./settings", true, true),
    );
    let merge = f.commit_with_parents("merge feature", &[&main1, &feat2]);
    f.git(&["update-ref", "refs/heads/main", &merge]);

    // A rebase: topic forks at the merge, main moves on, topic is replayed on the new main.
    let topic = |f: &mut Fixture, onto: &str, tag: &str| {
        f.git(&["read-tree", "--reset", "-u", onto]);
        f.write("src/topic.ts", "export const topic = 1;\n");
        let t1 = f.commit_with_parents(&format!("topic: add topic{tag}"), &[onto]);
        f.write(
            "src/topic.ts",
            "import { gone } from \"./gone\";\nexport const topic = gone;\n",
        );
        f.commit_with_parents(&format!("topic: topic uses gone{tag}"), &[&t1])
    };
    let old = topic(&mut f, &merge, "");
    f.git(&["branch", "topic-before-rebase", &old]);
    f.git(&["read-tree", "--reset", "-u", &merge]);
    f.write(
        "src/lib/util.ts",
        "export function util(n: number): number {\n  return n * 3;\n}\n",
    );
    let main2 = f.commit_with_parents("util triples", &[&merge]);
    let rebased = topic(&mut f, &main2, "");
    f.git(&["update-ref", "refs/heads/main", &rebased]);

    // A force-push: two commits pushed, then dropped and replaced by one.
    f.write(
        "src/wip.ts",
        "import { topic } from \"./topic\";\nexport const wip = topic;\n",
    );
    let wip1 = f.commit_with_parents("wip: start", &[&rebased]);
    f.write("src/wip.ts", "export const wip = 2;\n");
    let wip2 = f.commit_with_parents("wip: more", &[&wip1]);
    f.git(&["branch", "before-force-push", &wip2]);
    f.git(&["read-tree", "--reset", "-u", &rebased]);
    f.write(
        "src/done.ts",
        "import { topic } from \"./topic\";\nexport const done = topic;\n",
    );
    let done = f.commit_with_parents("done: the clean version", &[&rebased]);
    f.git(&["update-ref", "refs/heads/main", &done]);
    f.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    f.git(&["read-tree", "--reset", "-u", &done]);
}
