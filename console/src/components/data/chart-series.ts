/** One named, coloured data series shared by the line and bar charts. */
export interface ChartSeries {
  readonly name: string;
  readonly color: string;
  readonly data: readonly number[];
}
