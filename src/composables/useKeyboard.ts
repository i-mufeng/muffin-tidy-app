import { onMounted, onUnmounted } from "vue";
import { useProjectStore } from "../stores/project";

export function useKeyboard(cols: () => number) {
  const store = useProjectStore();

  function onKey(e: KeyboardEvent) {
    // ignore when typing in an input
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    // 大图浏览 / 导出对话框打开时，键盘交给对应组件，网格快捷键挂起
    if (store.viewerOpen) return;
    if (store.exportOpen) return;

    switch (e.key) {
      case "e":
      case "E":
        e.preventDefault();
        store.openExport();
        break;

      case "ArrowRight": e.preventDefault(); store.moveFocus(1); break;
      case "ArrowLeft":  e.preventDefault(); store.moveFocus(-1); break;
      case "ArrowDown":  e.preventDefault(); store.moveFocus(cols()); break;
      case "ArrowUp":    e.preventDefault(); store.moveFocus(-cols()); break;

      case " ":
        e.preventDefault();
        store.toggleMark(store.focusedIndex);
        break;

      case "d":
      case "D":
      case "Delete":
        e.preventDefault();
        store.removeFile(store.focusedIndex);
        break;

      case "z":
      case "Z":
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          store.undoLast();
        }
        break;
    }
  }

  onMounted(() => window.addEventListener("keydown", onKey));
  onUnmounted(() => window.removeEventListener("keydown", onKey));
}
