"""
Register custom Sphinx directives for rendering AMD SMI API documentation.

This extension reads a Doxygen tag file, collects API symbols and groups, and
emits Breathe directives to generate API reference content in Sphinx. It
supports Myst-flavoured Markdown sources via `myst-parser`.
"""

import xml.etree.ElementTree as XMLTree
from dataclasses import dataclass, field
from pathlib import Path
from typing import Final

from docutils import nodes
from docutils.parsers.rst import directives
from sphinx.application import Sphinx
from sphinx.config import Config
from sphinx.environment import BuildEnvironment
from sphinx.errors import ExtensionError
from sphinx.util.docutils import SphinxDirective


@dataclass(slots=True)
class DoxygenTagData:
    # groups are represented by `@defgroup` and its members by `@ingroup` in Doxygen comments
    groups: list[tuple[str, str]] = field(default_factory=list)
    # per-group members: maps group tag -> list of (member_kind, member_name)
    group_members: dict[str, list[tuple[str, str]]] = field(default_factory=dict)
    enums: list[str] = field(default_factory=list)
    defines: list[str] = field(default_factory=list)
    structs: list[str] = field(default_factory=list)
    unions: list[str] = field(default_factory=list)
    typedefs: list[str] = field(default_factory=list)


# Maps Doxygen tagfile member kind values to their Breathe directive names.
_MEMBER_KIND_TO_BREATHE: Final[dict[str, str]] = {
    "function": "doxygenfunction",
    "enumeration": "doxygenenum",
    "define": "doxygendefine",
    "typedef": "doxygentypedef",
}

# Maps the user-facing sub-kind token to the Doxygen tagfile member kind value.
_SUB_KIND_ALIASES: Final[dict[str, str]] = {
    "function": "function",
    "enum": "enumeration",
    "define": "define",
    "typedef": "typedef",
}


def render_groups_myst_markdown(groups: list[tuple[str, str]], heading_level: int) -> str:
    hashes = "#" * heading_level
    return "\n\n".join(
        # use raw HTML for the anchor because Myst's `()=` syntax formats the
        # anchor text
        f"<span id='{tag}'></span>\n\n"
        + f"{hashes} {title}\n\n"
        + f"```{{doxygengroup}} {tag}\n"
        + ":content-only:\n```"
        for tag, title in groups
    )


def render_groups_filtered_myst_markdown(
    groups: list[tuple[str, str]],
    group_members: dict[str, list[tuple[str, str]]],
    member_kind: str,
    breathe_directive: str,
    heading_level: int,
) -> str:
    hashes = "#" * heading_level
    parts = []
    for tag, title in groups:
        names = [
            name
            for kind, name in group_members.get(tag, [])
            if kind == member_kind
        ]
        if not names:
            continue
        members_text = render_members_myst_markdown(breathe_directive, names)
        parts.append(
            f"<span id='{tag}'></span>\n\n"
            + f"{hashes} {title}\n\n"
            + members_text
        )
    return "\n\n".join(parts)


def render_members_myst_markdown(directive_name: str, names: list[str]) -> str:
    return "\n\n".join(
        f"<span id='{name}'></span>\n\n```{{{directive_name}}} {name}\n```"
        for name in names
    )


class AmdsmiDoxygenDirective(SphinxDirective):
    """
    Render AMD SMI API content from a Doxygen tagfile via Breathe.

    Supported kinds:
    - groups
    - groups function  (only function members of each group)
    - groups enum      (only enum members of each group)
    - groups define    (only define members of each group)
    - groups typedef   (only typedef members of each group)
    - enums
    - defines
    - structs
    - unions
    - typedefs

    Usage in .md files:

        ```{amdsmi-doxygen}
        :kind: groups
        :heading-level: 2
        ```

        ```{amdsmi-doxygen}
        :kind: groups function
        :heading-level: 2
        ```
    """

    has_content = False
    required_arguments = 0
    optional_arguments = 0
    final_argument_whitespace = False
    option_spec = {
        "kind": directives.unchanged_required,
        "heading-level": directives.nonnegative_int,
    }

    def run(self) -> list[nodes.Node]:
        tokens = self.options["kind"].strip().lower().split()
        kind = tokens[0]
        sub_kind_token = tokens[1] if len(tokens) > 1 else None

        if sub_kind_token is not None and kind != "groups":
            raise self.error(
                "A sub-kind may only be specified when :kind: starts with 'groups'."
            )

        if sub_kind_token is not None and sub_kind_token not in _SUB_KIND_ALIASES:
            valid = ", ".join(sorted(_SUB_KIND_ALIASES))
            raise self.error(
                f"Invalid sub-kind {sub_kind_token!r} for 'groups'. "
                + f"Expected one of: {valid}"
            )

        if kind != "groups" and "heading-level" in self.options:
            raise self.error(":heading-level: is only valid when :kind: is 'groups'.")

        heading_level = self.options.get("heading-level", 2)
        if not 1 <= heading_level <= 6:
            raise self.error(":heading-level: must be between 1 and 6.")

        data = self._get_doxygen_tag_data()

        match kind:
            case "groups":
                if sub_kind_token is not None:
                    member_kind = _SUB_KIND_ALIASES[sub_kind_token]
                    breathe_directive = _MEMBER_KIND_TO_BREATHE[member_kind]
                    return self._render_groups_filtered(
                        data.groups, data.group_members, member_kind, breathe_directive, heading_level
                    )
                return self._render_groups(data.groups, heading_level)
            case "enums":
                return self._render_members("doxygenenum", data.enums)
            case "defines":
                return self._render_members("doxygendefine", data.defines)
            case "structs":
                return self._render_members("doxygenstruct", data.structs)
            case "unions":
                return self._render_members("doxygenunion", data.unions)
            case "typedefs":
                return self._render_members("doxygentypedef", data.typedefs)
            case _:
                raise self.error(
                    f"Invalid :kind: {kind!r}. Expected one of: "
                    + "groups, enums, defines, structs, unions, typedefs"
                )

    def _render_members(
        self, directive_name: str, names: list[str]
    ) -> list[nodes.Node]:
        if not names:
            return []

        text = render_members_myst_markdown(directive_name, names)

        return self.parse_text_to_nodes(text)

    def _render_groups(
        self, groups: list[tuple[str, str]], heading_level: int
    ) -> list[nodes.Node]:
        if not groups:
            return []

        text = render_groups_myst_markdown(groups, heading_level)

        return self.parse_text_to_nodes(text, allow_section_headings=True)

    def _render_groups_filtered(
        self,
        groups: list[tuple[str, str]],
        group_members: dict[str, list[tuple[str, str]]],
        member_kind: str,
        breathe_directive: str,
        heading_level: int,
    ) -> list[nodes.Node]:
        if not groups:
            return []

        text = render_groups_filtered_myst_markdown(
            groups, group_members, member_kind, breathe_directive, heading_level
        )
        return self.parse_text_to_nodes(text, allow_section_headings=True)

    def _get_doxygen_tag_data(self) -> DoxygenTagData:
        tagfile = Path(self.config.amdsmi_doxygen_tagfile)
        self.env.note_dependency(tagfile)

        cache = getattr(self.env, "_amdsmi_doxygen_tag_cache", None)
        if cache is None:
            cache = {}
            setattr(self.env, "_amdsmi_doxygen_tag_cache", cache)

        key = str(tagfile)
        data = cache.get(key)
        if data is not None:
            return data

        if not tagfile.exists():
            raise self.error(
                f"Doxygen tagfile not found: {tagfile}. "
                + "Check the GENERATE_TAGFILE option in Doxyfile and 'amdsmi_doxygen_tagfile' in conf.py."
            )
        try:
            root = XMLTree.parse(tagfile).getroot()
        except XMLTree.ParseError as e:
            raise self.error(f"Failed to parse Doxygen tagfile: {tagfile}") from e

        data = DoxygenTagData()

        for compound in root.findall("compound"):
            match compound.get("kind"):
                case "group":
                    group_tag = compound.findtext("name")
                    group_title = compound.findtext("title")
                    if group_tag and group_title:
                        data.groups.append((group_tag, group_title))
                        members: list[tuple[str, str]] = []

                        for member in compound.findall("member"):
                            m_kind = member.get("kind")
                            m_name = member.findtext("name")
                            if m_kind and m_name:
                                members.append((m_kind, m_name))
                        data.group_members[group_tag] = members

                case "file":
                    for member in compound.findall("member"):
                        member_kind = member.get("kind")
                        member_name = member.findtext("name")
                        if not member_name:
                            continue

                        if member_kind == "enumeration":
                            data.enums.append(member_name)
                        elif member_kind == "define":
                            data.defines.append(member_name)
                        elif member_kind == "typedef":
                            data.typedefs.append(member_name)

                case "struct":
                    struct_name = compound.findtext("name")
                    if struct_name:
                        data.structs.append(struct_name)

                case "union":
                    union_name = compound.findtext("name")
                    if union_name:
                        data.unions.append(union_name)

                case _:
                    continue

        cache[key] = data
        return data


def _validate_breathe_loaded(app: Sphinx, _config: Config) -> None:
    if "breathe" not in app.extensions:
        raise ExtensionError(
            "The 'amdsmi-doxygen' extension requires the 'breathe' Sphinx "
            + "extension to be enabled in conf.py."
        )


def _clear_tagfile_cache(
    _app: Sphinx, env: BuildEnvironment, _docnames: list[str]
) -> None:
    if hasattr(env, "_amdsmi_doxygen_tag_cache"):
        delattr(env, "_amdsmi_doxygen_tag_cache")


def setup(app: Sphinx) -> dict[str, bool]:
    app.add_config_value(
        "amdsmi_doxygen_tagfile",
        Path(app.confdir) / "doxygen" / "_out" / "tagfile.xml",
        "env",
        types=(str, Path),
    )
    app.add_directive("amdsmi-doxygen", AmdsmiDoxygenDirective)
    app.connect("config-inited", _validate_breathe_loaded, priority=600)
    app.connect("env-before-read-docs", _clear_tagfile_cache)

    return {
        "parallel_read_safe": True,
        "parallel_write_safe": True,
    }
