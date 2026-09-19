export type GreetResult = string;

export declare function greet(name: string): GreetResult;
export declare const add: (a: number, b: number) => number;

export declare class Counter {
  constructor(start?: number);
  value: number;
  bump(step?: number): number;
}
