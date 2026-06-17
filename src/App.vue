<template>
  <div class="app-shell">
    <TitleBar />
    <div class="view-host">
      <WorkspaceView v-if="store.phase === 'ready'" />
      <PreloadView v-else-if="store.phase === 'scanning' || store.phase === 'preloading'" />
      <HomeView v-else />
    </div>
  </div>
</template>

<script setup lang="ts">
import { useProjectStore } from "./stores/project";
import TitleBar from "./components/TitleBar.vue";
import HomeView from "./views/HomeView.vue";
import PreloadView from "./views/PreloadView.vue";
import WorkspaceView from "./views/WorkspaceView.vue";

const store = useProjectStore();
</script>

<style scoped>
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
}
.view-host {
  position: relative;
  flex: 1;
  min-height: 0;
}
</style>
