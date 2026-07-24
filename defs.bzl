load("@crates//:defs.bzl", "all_crate_deps")
load("@rules_rust//cargo/private:cargo_build_script_wrapper.bzl", "cargo_build_script")
load("@rules_rust//rust:defs.bzl", "rust_library", "rust_test")

def skia_rust_crate(
        name,
        crate_name,
        crate_features = [],
        build_script_data = [],
        compile_data = [],
        deps_extra = [],
        srcs_extra = [],
        test_data_extra = [],
        test_tags = [],
        target_compatible_with = []):
    """Defines one Cargo workspace library and its Rust tests for Bazel.

    Cargo.toml and Cargo.lock remain the dependency source of truth. The
    generated @crates repository supplies third-party dependencies and maps
    path dependencies to the matching workspace BUILD targets.
    """

    package_name = native.package_name()
    rustc_env = {
        "CARGO_MANIFEST_DIR": package_name,
    }
    build_deps = []

    if native.glob(["build.rs"], allow_empty = True):
        build_script_name = name + "-build-script"
        cargo_build_script(
            name = build_script_name,
            srcs = ["build.rs"],
            data = build_script_data,
            deps = all_crate_deps(build = True),
            edition = "2024",
            target_compatible_with = target_compatible_with,
            visibility = ["//visibility:private"],
        )
        build_deps.append(":" + build_script_name)

    rust_library(
        name = name,
        crate_features = crate_features,
        crate_name = crate_name,
        compile_data = compile_data,
        deps = all_crate_deps() + build_deps + deps_extra,
        edition = "2024",
        rustc_env = rustc_env,
        srcs = native.glob(["src/**/*.rs"]) + srcs_extra,
        target_compatible_with = target_compatible_with,
        visibility = ["//visibility:public"],
    )

    rust_test(
        name = name + "-unit-tests",
        crate = name,
        crate_features = crate_features,
        data = test_data_extra,
        deps = all_crate_deps(normal = True, normal_dev = True) + build_deps + deps_extra,
        rustc_env = rustc_env,
        tags = test_tags,
        target_compatible_with = target_compatible_with,
    )

    integration_srcs = native.glob(["tests/**/*.rs"], allow_empty = True)
    integration_data = native.glob(["tests/**"], allow_empty = True) + test_data_extra
    for test in native.glob(["tests/*.rs"], allow_empty = True):
        test_stem = test.removeprefix("tests/").removesuffix(".rs")
        rust_test(
            name = name + "-" + test_stem.replace("_", "-") + "-test",
            crate_name = test_stem.replace("-", "_"),
            crate_root = test,
            data = integration_data,
            deps = all_crate_deps(normal = True, normal_dev = True) + build_deps + deps_extra + [":" + name],
            edition = "2024",
            rustc_env = rustc_env,
            srcs = integration_srcs,
            tags = test_tags,
            target_compatible_with = target_compatible_with,
        )

    native.filegroup(
        name = "package-files",
        srcs = native.glob(
            ["**"],
            exclude = [
                "BUILD.bazel",
                "target/**",
            ],
            allow_empty = True,
        ),
        visibility = ["//visibility:public"],
    )
