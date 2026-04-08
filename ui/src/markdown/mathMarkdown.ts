import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";

/** `remark-math` before GFM (unified / react-markdown). */
export const markdownRemarkPlugins = [remarkMath, remarkGfm];

/** Runs after search-highlight (skips `code` math tokens) and before `rehype-sanitize`. */
export { rehypeKatex };
