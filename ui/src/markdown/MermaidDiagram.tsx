import { useEffect, useId, useRef, useState } from "react";

import styles from "./MermaidDiagram.module.scss";

type MermaidApi = typeof import("mermaid").default;

let mermaidLoader: Promise<MermaidApi> | null = null;

function loadMermaid(): Promise<MermaidApi> {
  if (mermaidLoader == null) {
    mermaidLoader = import("mermaid").then((mod) => {
      const mermaid = mod.default;
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "dark",
        darkMode: true,
      });
      return mermaid;
    });
  }
  return mermaidLoader;
}

interface MermaidDiagramProps {
  chart: string;
}

/** Renders a Mermaid / UML fenced block to SVG (lazy-loaded). */
export function MermaidDiagram({ chart }: MermaidDiagramProps) {
  const reactId = useId().replace(/:/g, "");
  const hostRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const definition = chart.trim();
    setPending(true);
    setError(null);
    if (hostRef.current != null) {
      hostRef.current.innerHTML = "";
    }

    void (async () => {
      try {
        const mermaid = await loadMermaid();
        const { svg, bindFunctions } = await mermaid.render(
          `tb-mermaid-${reactId}`,
          definition,
        );
        if (cancelled || hostRef.current == null) {
          return;
        }
        hostRef.current.innerHTML = svg;
        bindFunctions?.(hostRef.current);
        setPending(false);
      } catch (e) {
        if (cancelled) {
          return;
        }
        setError(e instanceof Error ? e.message : String(e));
        setPending(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [chart, reactId]);

  if (error != null) {
    return (
      <pre className={styles.error} data-testid="mermaid-error">
        {chart}
        {"\n\n"}
        {error}
      </pre>
    );
  }

  return (
    <div
      className={styles.diagram}
      data-testid="mermaid-diagram"
      aria-busy={pending}
    >
      {pending ? (
        <p className={styles.pending} data-print-hide>
          Rendering diagram…
        </p>
      ) : null}
      <div ref={hostRef} className={styles.svgHost} />
    </div>
  );
}
