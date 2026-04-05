import {
  ArcElement,
  Chart as ChartJS,
  Legend,
  Tooltip,
  type ChartData,
} from "chart.js";
import ChartDataLabels from "chartjs-plugin-datalabels";
import { Pie } from "react-chartjs-2";

ChartJS.register(ArcElement, Tooltip, Legend, ChartDataLabels);

interface StatsChartProps {
  data: ChartData<"pie">;
}

export default function StatsChart({ data }: StatsChartProps) {
  return (
    <Pie
      data={data}
      options={{
        plugins: {
          legend: { labels: { color: "#e8e4dc" } },
          datalabels: {
            color: "#e8e4dc",
            textAlign: "center",
            font: { size: 11, weight: 500 },
            formatter: (value, ctx) => {
              const arr = ctx.dataset.data as number[];
              const sum = arr.reduce((a, b) => a + b, 0);
              if (sum === 0 || typeof value !== "number") {
                return "";
              }
              const pct = (value / sum) * 100;
              if (pct < 7) {
                return "";
              }
              const label = ctx.chart.data.labels?.[ctx.dataIndex];
              const l = typeof label === "string" ? label : String(label ?? "");
              return `${l}\n${value}`;
            },
          },
        },
      }}
    />
  );
}
