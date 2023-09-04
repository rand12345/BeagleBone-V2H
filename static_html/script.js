const ampsSlider = document.getElementById('amps');
const ampsLabel = document.getElementById('ampsLabel');
const socLimitSlider = document.getElementById('soc_limit');
const socLimitLabel = document.getElementById('socLimitLabel');

const overlay = document.getElementById('overlay');
const messageBox = document.getElementById('messageBox');
const connectionStatus = document.getElementById('connectionStatus');
const eventsTable = document.getElementById('eventsTable').getElementsByTagName('tbody')[0];
const updateButton = document.getElementById('updateButton');

const addRowButton = document.getElementById('addRowButton');
addRowButton.addEventListener('click', () => addRow());

let eventData = [];
let currentlyEditingRow = null;


function showOverlay() {
  overlay.style.display = 'block';
  messageBox.style.display = 'block';
}

function hideOverlay() {
  overlay.style.display = 'none';
  messageBox.style.display = 'none';
}

function connectWebSocket() {
  showOverlay();

  const ws = new WebSocket('ws://10.0.1.177:5555');

  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    console.log("Incoming ws: ", message);
    if (message.Data) {
      updateDataTable(message.Data)
      // console.log(message.Data);
      addPlotData(message.Data);
    } else if (message.Mode) {
      updateButtons(message.Mode);
    } else if (message.Events) {
      eventData = message.Events; // Store the initial data
      populateTable(message.Events);
    }
  };

  ws.onopen = () => {
    hideOverlay();
    const initialRequestMessage1 = JSON.stringify({ cmd: "GetData" });
    const initialRequestMessage2 = JSON.stringify({ cmd: "GetMode" });
    const initialRequestMessage3 = JSON.stringify({ cmd: "GetEvents" });
    ws.send(initialRequestMessage1);
    ws.send(initialRequestMessage2);
    ws.send(initialRequestMessage3);

    setInterval(() => {
      ws.send(initialRequestMessage1);
      // console.log(initialRequestMessage1);
      ws.send(initialRequestMessage2);
      // console.log(initialRequestMessage2);
    }, 5000);
  };

  ws.onclose = () => {
    showOverlay();
    connectionStatus.innerText = "Connection lost. Attempting to reconnect...";

    // Attempt to reconnect after a delay
    setTimeout(connectWebSocket, 5000);
  };
  document.getElementById("chargeForm").addEventListener("submit", function (event) {
    event.preventDefault(); // Prevent the form from submitting

    // Gather form data
    const amps = parseInt(document.getElementById("amps").value);
    const eco = document.getElementById("eco").checked;
    const soc_limit = parseInt(document.getElementById("soc_limit").value);

    // Create JSON object
    const jsonCommand = {
      cmd: {
        SetMode: {
          Charge: {
            amps: amps,
            eco: eco,
            soc_limit: soc_limit
          }
        }
      }
    };
    console.log(JSON.stringify(jsonCommand, null, 2));
    ws.send(JSON.stringify(jsonCommand));
  });
  const buttons = document.querySelectorAll('.mode-button');
  buttons.forEach((button) => {
    button.addEventListener('click', () => {
      const mode = button.value;
      const message = { cmd: { SetMode: mode } };
      // const message = { SetMode: mode };
      ws.send(JSON.stringify(message));

      updateButtons(mode);
    });
  });
  updateButton.addEventListener('click', () => {
    const updatedData = Array.from(eventsTable.rows).map((row) => {
      return {
        time: row.cells[0].textContent,
        action: row.cells[1].textContent,
      };
    });

    const updateMessage = JSON.stringify({ cmd: { "SetEvents": updatedData } });

    console.log("Update table " + updateMessage);
    ws.send(updateMessage);
  });

}

connectWebSocket();

function updateDataTable(data) {
  const tbody = document.querySelector('#dataTable tbody');

  const newRow = document.createElement('tr');

  const localtimeCell = document.createElement('td');
  const currentTime = new Date().toLocaleTimeString();
  localtimeCell.textContent = currentTime;
  newRow.appendChild(localtimeCell);

  const socCell = document.createElement('td');
  socCell.textContent = data.soc;
  newRow.appendChild(socCell);

  const stateCell = document.createElement('td');
  stateCell.textContent = data.state;
  newRow.appendChild(stateCell);

  const tempCell = document.createElement('td');
  tempCell.textContent = data.temp;
  newRow.appendChild(tempCell);

  const fanCell = document.createElement('td');
  fanCell.textContent = data.fan;
  newRow.appendChild(fanCell);

  const wattsCell = document.createElement('td');
  wattsCell.textContent = data.ac_w;
  newRow.appendChild(wattsCell);

  tbody.insertBefore(newRow, tbody.firstChild);

  // If the tbody has too many rows, remove the last one
  if (tbody.children.length > 100) {
    tbody.removeChild(tbody.lastChild);
  }
}


function addRow() {
  const row = eventsTable.insertRow();
  const timeCell = row.insertCell(0);
  const actionCell = row.insertCell(1);
  const editCell = row.insertCell(2);
  const deleteCell = row.insertCell(3);


  // Default values for the new row
  const timePicker = document.createElement('input');
  timePicker.type = 'time';
  timePicker.value = '00:00:00';
  timeCell.innerHTML = '';
  timeCell.appendChild(timePicker);

  const actionDropdown = document.createElement('select');
  const actions = ['Charge', 'Discharge', 'Sleep', 'V2h', 'Eco'];
  actions.forEach((action) => {
    const option = document.createElement('option');
    option.value = action;
    option.textContent = action;
    actionDropdown.appendChild(option);
  });
  actionDropdown.value = 'Sleep';
  actionCell.innerHTML = '';
  actionCell.appendChild(actionDropdown);

  const editButton = document.createElement('button');
  editButton.textContent = 'Save';
  editButton.addEventListener('click', () => finishEditingRow(row));
  editCell.appendChild(editButton);

  // Simulate a click on the edit button for the new row
  editButton.click();

  const deleteButton = document.createElement('button');
  deleteButton.textContent = 'Delete';
  deleteButton.addEventListener('click', () => deleteRow(row));
  deleteCell.appendChild(deleteButton);
}

function populateTable(data) {
  eventsTable.innerHTML = '';

  data.forEach((event, index) => {
    const row = eventsTable.insertRow();
    const timeCell = row.insertCell(0);
    const actionCell = row.insertCell(1);
    const editCell = row.insertCell(2);
    const deleteCell = row.insertCell(3);

    timeCell.textContent = event.time;
    actionCell.textContent = event.action;

    const editButton = document.createElement('button');
    editButton.textContent = 'Edit';
    editButton.addEventListener('click', () => editRow(row, event));
    editCell.appendChild(editButton);

    const deleteButton = document.createElement('button');
    deleteButton.textContent = 'Delete';
    deleteButton.addEventListener('click', () => deleteRow(row, event));
    deleteCell.appendChild(deleteButton);
  });
}



function editRow(row, event) {
  // if (currentlyEditingRow) {
  //     finishEditingRow();
  // }
  if (event == null) {
    var event = '';
    event.time = '00:00:00';
    event.action = 'Sleep';
  };

  currentlyEditingRow = row;

  const timeCell = row.cells[0];
  const actionCell = row.cells[1];
  const editCell = row.cells[2];
  const editButton = document.createElement('button');
  editCell.innerHTML = ''
  editButton.textContent = 'Save';
  editButton.addEventListener('click', () => finishEditingRow(row, event));
  editCell.appendChild(editButton);


  // Create a time picker for the time cell
  const timePicker = document.createElement('input');
  timePicker.type = 'time';
  timePicker.step = 1;
  timePicker.value = event.time;
  timeCell.innerHTML = '';
  timeCell.appendChild(timePicker);

  // Create a dropdown for the action cell
  const actionDropdown = document.createElement('select');
  const actions = ['Charge', 'Discharge', 'Sleep', 'V2h', 'Eco'];
  actions.forEach((action) => {
    const option = document.createElement('option');
    option.value = action;
    option.textContent = action;
    actionDropdown.appendChild(option);
  });
  actionDropdown.value = event.action;
  actionCell.innerHTML = '';
  actionCell.appendChild(actionDropdown);

  // Disable the Update button
  updateButton.disabled = true;
  addRowButton.disabled = true;
}

function finishEditingRow(row, event) {
  // if (currentlyEditingRow) {
  const timeCell = row.cells[0];
  const actionCell = row.cells[1];
  const editCell = row.cells[2];
  const editButton = document.createElement('button');
  editCell.innerHTML = '';
  editButton.textContent = 'Edit';
  editButton.addEventListener('click', () => editRow(row, event));
  editCell.appendChild(editButton);
  // Remove the time picker and dropdown
  timeCell.textContent = timeCell.firstChild.value; //here
  actionCell.textContent = actionCell.firstChild.value;

  currentlyEditingRow = null;
  updateButton.disabled = false;
  addRowButton.disabled = false;
}

function deleteRow(row, event) {
  const confirmDelete = confirm('Are you sure you want to delete this event?');
  if (confirmDelete) {
    row.remove();
  }
}




function updateButtons(mode) {
  console.log("updateButtons " + mode);
  const buttons = document.querySelectorAll('.mode-button');
  buttons.forEach((button) => {
    if (button.value === mode) {
      button.classList.add('active');
      button.disabled = true;
    } else {
      button.classList.remove('active');
      button.disabled = false;
    }
  });
}




ampsSlider.addEventListener('input', () => {
  updateSliderLabel(ampsSlider, ampsLabel);
});

socLimitSlider.addEventListener('input', () => {
  updateSliderLabel(socLimitSlider, socLimitLabel);
});

function updateSliderLabel(slider, label) {
  const min = slider.min;
  const max = slider.max;
  const value = slider.value;

  label.innerText = value;
  label.style.left = `calc(${(value - min) / (max - min) * 100}% - 10px)`;

}

const plotContainer = document.getElementById('plotContainer');
let plotData = {
  dc_kw: [],
  amps: [],
  fan: [],
  requested_amps: [],
  soc: [],
  temp: [],
  meter_kw: [],
};

function addPlotData(data) {
  const time = new Date().toLocaleTimeString(); // Get current time

  // Append data points to respective arrays
  Object.keys(plotData).forEach(field => {
    if (field !== 'volts') {
      plotData[field].push({ x: time, y: data[field] });
    }
  });

  updatePlot();
}


function updatePlot() {
  const layout = {
    title: 'Real-time Data Plot',
    xaxis: {
      title: 'Time'
    },
    yaxis: {
      title: 'Value'
    },
    yaxis2: {
      title: 'SoC, Fan and Temp',
      overlaying: 'y',
      side: 'right',
      range: [0, 100]
    },
    legend: {
      x: 0.0, // Center the legend horizontally
      y: -0.4, // Move the legend to the bottom
      orientation: 'h' // Arrange the legend horizontally
    }
  };

  const plotConfig = {
    responsive: true
  };

  const traces = Object.keys(plotData).map(field => {
    if (field === 'soc' || field === 'temp' || field === 'fan') {
      return {
        x: plotData[field].map(dataPoint => dataPoint.x),
        y: plotData[field].map(dataPoint => dataPoint.y),
        name: field,
        type: 'line',
        yaxis: 'y2' // Associate with the second y-axis
      };
    } else {
      return {
        x: plotData[field].map(dataPoint => dataPoint.x),
        y: plotData[field].map(dataPoint => dataPoint.y),
        name: field,
        type: 'line'
      };
    }
  });

  const data = traces;

  Plotly.newPlot(plotContainer, data, layout, plotConfig);
}




