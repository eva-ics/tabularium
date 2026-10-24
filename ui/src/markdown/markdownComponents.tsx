import {
  Children,
  isValidElement,
  type ComponentPropsWithoutRef,
  type ReactNode,
} from "react";

import { MermaidDiagram } from "./MermaidDiagram";

const DIAGRAM_LANGS = new Set(["mermaid", "uml"]);

function codeText(children: ReactNode): string {
  return String(children).replace(/\n$/, "");
}

function languageFromClassName(className: string | undefined): string | null {
  const match = /language-([a-z0-9_+-]+)/i.exec(className ?? "");
  return match?.[1]?.toLowerCase() ?? null;
}

function isDiagramFence(className: string | undefined): boolean {
  const lang = languageFromClassName(className);
  return lang != null && DIAGRAM_LANGS.has(lang);
}

type CodeProps = ComponentPropsWithoutRef<"code"> & {
  children?: ReactNode;
};

type PreProps = ComponentPropsWithoutRef<"pre"> & {
  children?: ReactNode;
};

/** Shared `code` / `pre` handlers: Mermaid UML fences → SVG; else default HTML. */
export function markdownDiagramComponents(): {
  code: (props: CodeProps) => ReactNode;
  pre: (props: PreProps) => ReactNode;
} {
  return {
    code({ className, children, ...props }: CodeProps) {
      if (isDiagramFence(className)) {
        return <MermaidDiagram chart={codeText(children)} />;
      }
      return (
        <code className={className} {...props}>
          {children}
        </code>
      );
    },
    pre({ children, ...props }: PreProps) {
      const items = Children.toArray(children);
      if (
        items.length === 1 &&
        isValidElement(items[0]) &&
        items[0].type === MermaidDiagram
      ) {
        return items[0];
      }
      return <pre {...props}>{children}</pre>;
    },
  };
}
