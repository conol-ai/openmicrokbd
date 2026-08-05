import type { LaserApi } from "../../shared/laser";

declare global {
  interface Window {
    laser: LaserApi;
  }
}

export {};
